//! Contracts independent of the GUI toolkit and browser engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub url: String,
    pub loading: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserSnapshot {
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserCommand {
    NewTab { url: String },
    CloseTab(TabId),
    ActivateTab(TabId),
    Navigate { tab: TabId, input: String },
    Back(TabId),
    Forward(TabId),
    Reload(TabId),
}

#[derive(Debug, PartialEq, Eq)]
pub enum BrowserError {
    TabNotFound(TabId),
    Engine(String),
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BrowserError {}

pub trait BrowserEngine {
    fn open(&mut self, id: TabId, url: &str) -> Result<(), BrowserError>;
    fn close(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn navigate(&mut self, id: TabId, url: &str) -> Result<(), BrowserError>;
    fn back(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn forward(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn reload(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn activate(&mut self, id: TabId) -> Result<(), BrowserError>;
}

/// Converts an address-bar input into a URL. This is intentionally small;
/// a search provider and full URL parser belong to a later phase.
pub fn normalize_address(input: &str) -> String {
    let value = input.trim();
    if value.is_empty() { return "about:blank".into(); }
    if value.starts_with("https://") || value.starts_with("http://") || value == "about:blank" {
        value.into()
    } else if value.contains(' ') {
        format!("https://www.google.com/search?q={}", value.split_whitespace().collect::<Vec<_>>().join("+"))
    } else {
        format!("https://{value}")
    }
}
