use browser_contracts::{
    BrowserCommand, BrowserEngine, BrowserError, BrowserEvent, BrowserSnapshot, BrowserTick, Tab,
    TabId, normalize_address,
};

pub struct Browser<E: BrowserEngine> {
    engine: E,
    tabs: Vec<Tab>,
    active: Option<TabId>,
    next_id: u64,
}

impl<E: BrowserEngine> Browser<E> {
    pub fn new(engine: E) -> Self {
        Self {
            engine,
            tabs: Vec::new(),
            active: None,
            next_id: 1,
        }
    }

    pub fn snapshot(&self) -> BrowserSnapshot {
        BrowserSnapshot {
            tabs: self.tabs.clone(),
            active: self.active,
        }
    }

    pub fn dispatch(&mut self, command: BrowserCommand) -> Result<(), BrowserError> {
        match command {
            BrowserCommand::NewTab { url } => {
                let id = TabId(self.next_id);
                let url = normalize_address(&url);
                self.engine.open(id, &url)?;
                if let Err(error) = self.engine.activate(id) {
                    let _ = self.engine.close(id);
                    return Err(error);
                }
                self.next_id += 1;
                self.tabs.push(Tab {
                    id,
                    title: "New tab".into(),
                    url,
                    loading: false,
                });
                self.active = Some(id);
            }
            BrowserCommand::CloseTab(id) => {
                let index = self.index(id)?;
                let next_active = if self.active == Some(id) {
                    self.tabs.iter().find(|tab| tab.id != id).map(|tab| tab.id)
                } else {
                    self.active
                };
                if self.active == Some(id) {
                    if let Some(next) = next_active {
                        self.engine.activate(next)?;
                    }
                }
                self.engine.close(id)?;
                self.tabs.remove(index);
                self.active = next_active;
            }
            BrowserCommand::ActivateTab(id) => {
                self.index(id)?;
                self.engine.activate(id)?;
                self.active = Some(id);
            }
            BrowserCommand::Navigate { tab, input } => {
                let index = self.index(tab)?;
                let url = normalize_address(&input);
                self.engine.navigate(tab, &url)?;
                self.tabs[index].url = url;
                self.tabs[index].title = "Loading...".into();
                self.tabs[index].loading = true;
            }
            BrowserCommand::Back(id) => {
                self.index(id)?;
                self.engine.back(id)?;
            }
            BrowserCommand::Forward(id) => {
                self.index(id)?;
                self.engine.forward(id)?;
            }
            BrowserCommand::Reload(id) => {
                self.index(id)?;
                self.engine.reload(id)?;
            }
            BrowserCommand::Resize { tab, width, height } => {
                self.index(tab)?;
                if width > 0 && height > 0 {
                    self.engine.resize(tab, width, height)?;
                }
            }
            BrowserCommand::Pointer { tab, x, y, action } => {
                self.index(tab)?;
                self.engine.pointer(tab, x, y, action)?;
            }
            BrowserCommand::Scroll { tab, x, y, dx, dy } => {
                self.index(tab)?;
                self.engine.scroll(tab, x, y, dx, dy)?;
            }
        }
        Ok(())
    }

    /// Apply asynchronous engine events on the UI thread, without exposing CEF to the frontend.
    pub fn tick(&mut self) -> BrowserTick {
        let changes = self.engine.poll();
        for event in changes.events {
            match event {
                BrowserEvent::UrlChanged { tab, url } => {
                    if let Some(item) = self.tabs.iter_mut().find(|item| item.id == tab) {
                        item.url = url;
                    }
                }
                BrowserEvent::TitleChanged { tab, title } => {
                    if let Some(item) = self.tabs.iter_mut().find(|item| item.id == tab) {
                        item.title = title;
                    }
                }
                BrowserEvent::LoadingChanged { tab, loading } => {
                    if let Some(item) = self.tabs.iter_mut().find(|item| item.id == tab) {
                        item.loading = loading;
                    }
                }
                BrowserEvent::Closed { .. } => {
                    // A close acknowledgment belongs to the engine lifecycle, not tab removal.
                }
            }
        }
        BrowserTick {
            snapshot: self.snapshot(),
            frames: changes.frames,
        }
    }

    /// Request closure of all tabs before the Chromium runtime is torn down.
    pub fn close_all(&mut self) -> Result<(), BrowserError> {
        while let Some(tab) = self.tabs.first() {
            let id = tab.id;
            self.dispatch(BrowserCommand::CloseTab(id))?;
        }
        Ok(())
    }

    fn index(&self, id: TabId) -> Result<usize, BrowserError> {
        self.tabs
            .iter()
            .position(|tab| tab.id == id)
            .ok_or(BrowserError::TabNotFound(id))
    }
}

/// Deterministic engine for core/frontend development without a Chromium runtime.
#[derive(Default)]
pub struct MockEngine {
    pub events: Vec<String>,
}
impl BrowserEngine for MockEngine {
    fn open(&mut self, id: TabId, url: &str) -> Result<(), BrowserError> {
        self.events.push(format!("open:{}:{url}", id.0));
        Ok(())
    }
    fn close(&mut self, id: TabId) -> Result<(), BrowserError> {
        self.events.push(format!("close:{}", id.0));
        Ok(())
    }
    fn navigate(&mut self, id: TabId, url: &str) -> Result<(), BrowserError> {
        self.events.push(format!("navigate:{}:{url}", id.0));
        Ok(())
    }
    fn back(&mut self, _: TabId) -> Result<(), BrowserError> {
        Ok(())
    }
    fn forward(&mut self, _: TabId) -> Result<(), BrowserError> {
        Ok(())
    }
    fn reload(&mut self, _: TabId) -> Result<(), BrowserError> {
        Ok(())
    }
    fn activate(&mut self, _: TabId) -> Result<(), BrowserError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_switching_and_closing_tabs() {
        let mut browser = Browser::new(MockEngine::default());
        browser
            .dispatch(BrowserCommand::NewTab {
                url: "example.org".into(),
            })
            .unwrap();
        browser
            .dispatch(BrowserCommand::NewTab {
                url: "rust-lang.org".into(),
            })
            .unwrap();
        assert_eq!(browser.snapshot().active, Some(TabId(2)));
        browser
            .dispatch(BrowserCommand::ActivateTab(TabId(1)))
            .unwrap();
        browser
            .dispatch(BrowserCommand::CloseTab(TabId(1)))
            .unwrap();
        assert_eq!(browser.snapshot().active, Some(TabId(2)));
        assert_eq!(browser.snapshot().tabs.len(), 1);
    }
    #[test]
    fn failed_activation_preserves_state() {
        let mut browser = Browser::new(MockEngine::default());
        browser
            .dispatch(BrowserCommand::NewTab {
                url: "example.org".into(),
            })
            .unwrap();
        assert_eq!(
            browser.dispatch(BrowserCommand::ActivateTab(TabId(42))),
            Err(BrowserError::TabNotFound(TabId(42)))
        );
        assert_eq!(browser.snapshot().active, Some(TabId(1)));
    }
    #[test]
    fn address_normalization() {
        assert_eq!(normalize_address(" example.org "), "https://example.org");
        assert_eq!(
            normalize_address("https://example.org"),
            "https://example.org"
        );
        assert_eq!(
            normalize_address("a test"),
            "https://www.google.com/search?q=a+test"
        );
    }
    #[test]
    fn closing_all_tabs_is_idempotent() {
        let mut browser = Browser::new(MockEngine::default());
        browser
            .dispatch(BrowserCommand::NewTab {
                url: "example.org".into(),
            })
            .unwrap();
        browser
            .dispatch(BrowserCommand::NewTab {
                url: "example.net".into(),
            })
            .unwrap();
        browser.close_all().unwrap();
        browser.close_all().unwrap();
        assert!(browser.snapshot().tabs.is_empty());
        assert_eq!(browser.snapshot().active, None);
    }
    #[test]
    fn engine_events_update_tab_metadata() {
        struct EventEngine;
        impl BrowserEngine for EventEngine {
            fn open(&mut self, _: TabId, _: &str) -> Result<(), BrowserError> {
                Ok(())
            }
            fn close(&mut self, _: TabId) -> Result<(), BrowserError> {
                Ok(())
            }
            fn navigate(&mut self, _: TabId, _: &str) -> Result<(), BrowserError> {
                Ok(())
            }
            fn back(&mut self, _: TabId) -> Result<(), BrowserError> {
                Ok(())
            }
            fn forward(&mut self, _: TabId) -> Result<(), BrowserError> {
                Ok(())
            }
            fn reload(&mut self, _: TabId) -> Result<(), BrowserError> {
                Ok(())
            }
            fn activate(&mut self, _: TabId) -> Result<(), BrowserError> {
                Ok(())
            }
            fn poll(&mut self) -> browser_contracts::EngineUpdates {
                browser_contracts::EngineUpdates {
                    events: vec![
                        BrowserEvent::TitleChanged {
                            tab: TabId(1),
                            title: "Example".into(),
                        },
                        BrowserEvent::UrlChanged {
                            tab: TabId(1),
                            url: "https://example.org/".into(),
                        },
                    ],
                    frames: Vec::new(),
                }
            }
        }
        let mut browser = Browser::new(EventEngine);
        browser
            .dispatch(BrowserCommand::NewTab {
                url: "example.org".into(),
            })
            .unwrap();
        let snapshot = browser.tick().snapshot;
        assert_eq!(snapshot.tabs[0].title, "Example");
        assert_eq!(snapshot.tabs[0].url, "https://example.org/");
    }
}
