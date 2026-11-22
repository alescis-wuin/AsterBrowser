//! Local, opt-in benchmark transport. No dependency on CEF, Slint or a renderer.
//! Commands are executed by the application's UI thread, never by this socket worker.
use browser_contracts::BrowserSnapshot;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_REQUEST_BYTES: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Ping,
    Snapshot,
    Mark {
        label: String,
    },
    Navigate {
        url: String,
    },
    NewTab {
        url: String,
    },
    CloseTab {
        tab: Option<u64>,
    },
    ActivateTab {
        tab: u64,
    },
    Back,
    Forward,
    Reload,
    Scroll {
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
    },
    Pointer {
        x: i32,
        y: i32,
        kind: String,
        button: Option<String>,
    },
    Text {
        text: String,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Quit,
}

#[derive(Debug, Deserialize)]
struct WireRequest {
    id: u64,
    token: String,
}

#[derive(Debug)]
pub struct ControlRequest {
    pub id: u64,
    pub action: Action,
    pub answer: Sender<ControlReply>,
}

#[derive(Debug, Serialize)]
pub struct ControlReply {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
}
impl ControlReply {
    pub fn ok(id: u64, snapshot: Option<BrowserSnapshot>) -> Self {
        Self {
            id,
            ok: true,
            error: None,
            snapshot: snapshot.map(Snapshot::from),
        }
    }
    pub fn error(id: u64, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            error: Some(message.into()),
            snapshot: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub active: Option<u64>,
    pub tabs: Vec<Tab>,
}
#[derive(Debug, Serialize)]
pub struct Tab {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub loading: bool,
}
impl From<BrowserSnapshot> for Snapshot {
    fn from(value: BrowserSnapshot) -> Self {
        Self {
            active: value.active.map(|tab| tab.0),
            tabs: value
                .tabs
                .into_iter()
                .map(|tab| Tab {
                    id: tab.id.0,
                    url: tab.url,
                    title: tab.title,
                    loading: tab.loading,
                })
                .collect(),
        }
    }
}

/// A loopback-only TCP control server. Port and token are supplied exclusively
/// by the benchmark runner. There is no listener in a normal Aster launch.
pub struct ControlServer {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ControlServer {
    pub fn start(
        port: u16,
        token: String,
        notify_ui: Arc<dyn Fn() + Send + Sync>,
    ) -> io::Result<(Self, Receiver<ControlRequest>)> {
        if port < 1024 || token.len() < 32 || !token.is_ascii() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "benchmark port must be >=1024 and token must have at least 32 ASCII characters",
            ));
        }
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        listener.set_nonblocking(true)?;
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let _ = client.set_read_timeout(Some(Duration::from_millis(500)));
                        let _ = client.set_write_timeout(Some(Duration::from_secs(2)));
                        if let Err(error) =
                            handle_client(client, &token, &sender, &notify_ui, &worker_stop)
                        {
                            eprintln!("[aster:benchmark] connection_error={error}");
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => {
                        eprintln!("[aster:benchmark] listener_error={error}");
                        break;
                    }
                }
            }
        });
        Ok((
            Self {
                stop,
                worker: Some(worker),
            },
            receiver,
        ))
    }
}

fn handle_client(
    stream: TcpStream,
    token: &str,
    sender: &Sender<ControlRequest>,
    notify_ui: &Arc<dyn Fn() + Send + Sync>,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    while !stop.load(Ordering::Acquire) {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(bytes) if bytes > MAX_REQUEST_BYTES => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request too large",
                ));
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
        let reply = match serde_json::from_str::<WireRequest>(&line) {
            Ok(wire) if wire.token == token => {
                // `Action` is internally tagged, so deserialize it separately:
                // serde(flatten) does not support enum payloads.
                match serde_json::from_str::<Action>(&line) {
                    Ok(action) => {
                        let (reply_tx, reply_rx) = mpsc::channel();
                        if sender
                            .send(ControlRequest {
                                id: wire.id,
                                action,
                                answer: reply_tx,
                            })
                            .is_err()
                        {
                            ControlReply::error(wire.id, "UI command receiver unavailable")
                        } else {
                            notify_ui();
                            reply_rx.recv_timeout(REQUEST_TIMEOUT).unwrap_or_else(|_| {
                                ControlReply::error(wire.id, "UI command timed out")
                            })
                        }
                    }
                    Err(error) => ControlReply::error(wire.id, format!("invalid action: {error}")),
                }
            }
            Ok(wire) => ControlReply::error(wire.id, "unauthorized"),
            Err(error) => ControlReply::error(0, format!("invalid request: {error}")),
        };
        let stream = reader.get_mut();
        let encoded = serde_json::to_vec(&reply)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        stream.write_all(&encoded)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
    }
    Ok(())
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_parse_without_a_gui() {
        let wire: WireRequest = serde_json::from_str(
            r#"{"id":8,"token":"secret","action":"scroll","x":2,"y":3,"dx":0,"dy":-100}"#,
        )
        .unwrap();
        assert_eq!(wire.id, 8);
        let action: Action = serde_json::from_str(
            r#"{"id":8,"token":"secret","action":"scroll","x":2,"y":3,"dx":0,"dy":-100}"#,
        )
        .unwrap();
        assert!(matches!(action, Action::Scroll { dy: -100, .. }));
        let sample = BrowserSnapshot {
            active: None,
            tabs: vec![],
        };
        let reply = serde_json::to_string(&ControlReply::ok(2, Some(sample))).unwrap();
        assert!(reply.contains("\"tabs\":[]"));
    }
    #[test]
    fn malformed_actions_are_rejected() {
        assert!(
            serde_json::from_str::<Action>(
                r#"{"id":9,"token":"a","action":"execute_shell","command":"rm"}"#
            )
            .is_err()
        );
    }
}
