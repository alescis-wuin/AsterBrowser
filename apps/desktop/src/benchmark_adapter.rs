//! Optional local benchmark adapter at the composition root.
//! The transport crate is GUI/CEF-independent; commands execute on the UI thread.
use benchmark_control::{Action, ControlReply, ControlServer};
use browser_contracts::{
    BrowserCommand, BrowserEngine, BrowserKey, InputModifiers, KeyAction, KeyInput, PointerAction,
    PointerButton, TabId,
};
use browser_core::Browser;
use frontend_slint::{ExternalUiAction, FrontendConfig};
use std::{
    cell::RefCell,
    error::Error,
    rc::Rc,
    sync::{Arc, mpsc::TryRecvError},
};

pub fn install<E: BrowserEngine + 'static>(
    browser: &Rc<RefCell<Browser<E>>>,
    frontend: FrontendConfig,
) -> Result<(FrontendConfig, Option<ControlServer>), Box<dyn Error>> {
    let port_text = match std::env::var("ASTER_BENCH_PORT") {
        Ok(port) => port,
        Err(std::env::VarError::NotPresent) => return Ok((frontend, None)),
        Err(error) => return Err(error.into()),
    };
    let port: u16 = port_text.parse()?;
    let token = std::env::var("ASTER_BENCH_TOKEN")?;
    let (server, receiver) = ControlServer::start(
        port,
        token,
        Arc::new(frontend_slint::notify_external_commands),
    )?;
    let browser = Rc::clone(browser);
    let frontend = frontend.with_external_control(Rc::new(move || {
        let mut ui_actions = Vec::new();
        loop {
            match receiver.try_recv() {
                Ok(request) => {
                    let (reply, action) = execute(&browser, request.id, request.action);
                    if let Some(action) = action {
                        ui_actions.push(action);
                    }
                    // Acknowledgement is sent after dispatch, before the next Slint redraw.
                    let _ = request.answer.send(reply);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        ui_actions
    }));
    eprintln!("[aster:benchmark] internal_control=enabled bind=127.0.0.1:{port}");
    Ok((frontend, Some(server)))
}

fn execute<E: BrowserEngine>(
    browser: &Rc<RefCell<Browser<E>>>,
    id: u64,
    action: Action,
) -> (ControlReply, Option<ExternalUiAction>) {
    let mut browser = browser.borrow_mut();
    let snapshot = browser.snapshot();
    let active = snapshot.active;
    let mut window_action = None;
    let outcome: Result<(), String> = (|| {
        let command: Option<BrowserCommand> = match action {
            Action::Ping | Action::Snapshot => None,
            Action::Mark { label } => {
                if label.is_empty()
                    || label.len() > 64
                    || !label
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                {
                    return Err("invalid benchmark marker".into());
                }
                eprintln!("[aster:benchmark] mark={label}");
                None
            }
            Action::Navigate { url } => Some(BrowserCommand::Navigate {
                tab: active.ok_or("no active tab")?,
                input: url,
            }),
            Action::NewTab { url } => Some(BrowserCommand::NewTab { url }),
            Action::CloseTab { tab } => Some(BrowserCommand::CloseTab(
                tab.map(TabId).or(active).ok_or("no tab to close")?,
            )),
            Action::ActivateTab { tab } => Some(BrowserCommand::ActivateTab(TabId(tab))),
            Action::Back => Some(BrowserCommand::Back(active.ok_or("no active tab")?)),
            Action::Forward => Some(BrowserCommand::Forward(active.ok_or("no active tab")?)),
            Action::Reload => Some(BrowserCommand::Reload(active.ok_or("no active tab")?)),
            Action::Scroll { x, y, dx, dy } => {
                let tab = active.ok_or("no active tab")?;
                window_action = Some(ExternalUiAction::Input { tab });
                Some(BrowserCommand::Scroll {
                    tab,
                    x,
                    y,
                    dx,
                    dy,
                    modifiers: InputModifiers::default(),
                })
            }
            Action::Pointer { x, y, kind, button } => {
                let tab = active.ok_or("no active tab")?;
                let action = match kind.as_str() {
                    "down" => PointerAction::Down,
                    "up" => PointerAction::Up,
                    "move" => PointerAction::Move,
                    "cancel" => PointerAction::Cancel,
                    _ => return Err("pointer kind must be down/up/move/cancel".into()),
                };
                let button = match button.as_deref() {
                    None => None,
                    Some("left") => Some(PointerButton::Left),
                    Some("middle") => Some(PointerButton::Middle),
                    Some("right") => Some(PointerButton::Right),
                    _ => return Err("unknown pointer button".into()),
                };
                window_action = Some(ExternalUiAction::Input { tab });
                Some(BrowserCommand::Pointer {
                    tab,
                    x,
                    y,
                    action,
                    button,
                    modifiers: InputModifiers::default(),
                })
            }
            Action::Text { text } => {
                let tab = active.ok_or("no active tab")?;
                if text.len() > 4096 {
                    return Err("text too long".into());
                }
                window_action = Some(ExternalUiAction::Input { tab });
                for ch in text.chars() {
                    let input = KeyInput {
                        key: BrowserKey::Text(ch.to_string()),
                        action: KeyAction::Press,
                        modifiers: InputModifiers::default(),
                        repeat: false,
                    };
                    browser
                        .dispatch(BrowserCommand::Key {
                            tab,
                            input: input.clone(),
                        })
                        .map_err(|e| e.to_string())?;
                    browser
                        .dispatch(BrowserCommand::Key {
                            tab,
                            input: KeyInput {
                                action: KeyAction::Release,
                                ..input
                            },
                        })
                        .map_err(|e| e.to_string())?;
                }
                None
            }
            Action::Resize { width, height } => {
                if !(320..=3840).contains(&width) || !(240..=2160).contains(&height) {
                    return Err("resize must be within 320..3840 x 240..2160".into());
                }
                window_action = Some(ExternalUiAction::Resize { width, height });
                None
            }
            Action::Quit => {
                window_action = Some(ExternalUiAction::Quit);
                None
            }
        };
        if let Some(command) = command {
            browser.dispatch(command).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => (
            ControlReply::ok(id, Some(browser.snapshot())),
            window_action,
        ),
        Err(error) => (ControlReply::error(id, error), None),
    }
}
