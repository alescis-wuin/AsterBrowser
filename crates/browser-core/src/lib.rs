use browser_contracts::{normalize_address, BrowserCommand, BrowserEngine, BrowserError, BrowserSnapshot, Tab, TabId};

pub struct Browser<E: BrowserEngine> {
    engine: E,
    tabs: Vec<Tab>,
    active: Option<TabId>,
    next_id: u64,
}
impl<E: BrowserEngine> Browser<E> {
    pub fn new(engine: E) -> Self { Self { engine, tabs: Vec::new(), active: None, next_id: 1 } }
    pub fn snapshot(&self) -> BrowserSnapshot {
        BrowserSnapshot { tabs: self.tabs.clone(), active: self.active }
    }
    pub fn dispatch(&mut self, command: BrowserCommand) -> Result<(), BrowserError> {
        match command {
            BrowserCommand::NewTab { url } => {
                let id = TabId(self.next_id);
                let url = normalize_address(&url);
                self.engine.open(id, &url)?;
                if let Err(e) = self.engine.activate(id) {
                    let _ = self.engine.close(id);
                    return Err(e);
                }
                self.next_id += 1;
                self.tabs.push(Tab { id, title: "Nouvel onglet".into(), url, loading: false });
                self.active = Some(id);
            }
            BrowserCommand::CloseTab(id) => {
                let index = self.index(id)?;
                let next_active = if self.active == Some(id) {
                    self.tabs.iter().find(|t| t.id != id).map(|t| t.id)
                } else { self.active };
                if let Some(next) = next_active.filter(|_| self.active == Some(id)) {
                    self.engine.activate(next)?;
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
                self.tabs[index].title = "Chargement…".into();
            }
            BrowserCommand::Back(id) => { self.index(id)?; self.engine.back(id)?; }
            BrowserCommand::Forward(id) => { self.index(id)?; self.engine.forward(id)?; }
            BrowserCommand::Reload(id) => { self.index(id)?; self.engine.reload(id)?; }
        }
        Ok(())
    }
    fn index(&self, id: TabId) -> Result<usize, BrowserError> {
        self.tabs.iter().position(|t| t.id == id).ok_or(BrowserError::TabNotFound(id))
    }
}

/// Deterministic, headless engine for core/UI development.
#[derive(Default)]
pub struct MockEngine {
    pub events: Vec<String>,
}
impl BrowserEngine for MockEngine {
    fn open(&mut self, id: TabId, url: &str) -> Result<(), BrowserError> { self.events.push(format!("open:{}:{url}", id.0)); Ok(()) }
    fn close(&mut self, id: TabId) -> Result<(), BrowserError> { self.events.push(format!("close:{}", id.0)); Ok(()) }
    fn navigate(&mut self, id: TabId, url: &str) -> Result<(), BrowserError> { self.events.push(format!("navigate:{}:{url}", id.0)); Ok(()) }
    fn back(&mut self, _: TabId) -> Result<(), BrowserError> { Ok(()) }
    fn forward(&mut self, _: TabId) -> Result<(), BrowserError> { Ok(()) }
    fn reload(&mut self, _: TabId) -> Result<(), BrowserError> { Ok(()) }
    fn activate(&mut self, _: TabId) -> Result<(), BrowserError> { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_switching_and_closing_tabs() {
        let mut browser = Browser::new(MockEngine::default());
        browser.dispatch(BrowserCommand::NewTab { url: "example.org".into() }).unwrap();
        browser.dispatch(BrowserCommand::NewTab { url: "rust-lang.org".into() }).unwrap();
        assert_eq!(browser.snapshot().active, Some(TabId(2)));
        browser.dispatch(BrowserCommand::ActivateTab(TabId(1))).unwrap();
        browser.dispatch(BrowserCommand::CloseTab(TabId(1))).unwrap();
        assert_eq!(browser.snapshot().active, Some(TabId(2)));
        assert_eq!(browser.snapshot().tabs.len(), 1);
    }
    #[test]
    fn failed_activation_preserves_state() {
        let mut browser = Browser::new(MockEngine::default());
        browser.dispatch(BrowserCommand::NewTab { url: "example.org".into() }).unwrap();
        assert_eq!(browser.dispatch(BrowserCommand::ActivateTab(TabId(42))), Err(BrowserError::TabNotFound(TabId(42))));
        assert_eq!(browser.snapshot().active, Some(TabId(1)));
    }
    #[test]
    fn address_normalization() {
        assert_eq!(normalize_address(" example.org "), "https://example.org");
        assert_eq!(normalize_address("https://example.org"), "https://example.org");
        assert_eq!(normalize_address("a test"), "https://www.google.com/search?q=a+test");
    }
}
