//! Browser protocols shared by any rendering engine and any frontend.
//! These types must not depend on a particular GUI toolkit or CEF.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerAction {
    Down,
    Up,
    Move,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserCommand {
    NewTab {
        url: String,
    },
    CloseTab(TabId),
    ActivateTab(TabId),
    Navigate {
        tab: TabId,
        input: String,
    },
    Back(TabId),
    Forward(TabId),
    Reload(TabId),
    Resize {
        tab: TabId,
        width: u32,
        height: u32,
    },
    Pointer {
        tab: TabId,
        x: i32,
        y: i32,
        action: PointerAction,
    },
    Scroll {
        tab: TabId,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
    },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserEvent {
    UrlChanged { tab: TabId, url: String },
    TitleChanged { tab: TabId, title: String },
    LoadingChanged { tab: TabId, loading: bool },
    Closed { tab: TabId },
}

/// An immutable BGRA8 CEF paint frame. Consumers may convert this to their own texture type.
#[derive(Debug)]
pub struct PageFrame {
    pub tab: TabId,
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

#[derive(Default, Debug)]
pub struct EngineUpdates {
    pub events: Vec<BrowserEvent>,
    /// Each adapter should retain only its latest unconsumed frame per tab.
    pub frames: Vec<PageFrame>,
}

pub struct BrowserTick {
    pub snapshot: BrowserSnapshot,
    pub frames: Vec<PageFrame>,
}

pub trait BrowserEngine {
    fn open(&mut self, id: TabId, url: &str) -> Result<(), BrowserError>;
    fn close(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn navigate(&mut self, id: TabId, url: &str) -> Result<(), BrowserError>;
    fn back(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn forward(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn reload(&mut self, id: TabId) -> Result<(), BrowserError>;
    fn activate(&mut self, id: TabId) -> Result<(), BrowserError>;

    fn resize(&mut self, _id: TabId, _width: u32, _height: u32) -> Result<(), BrowserError> {
        Ok(())
    }
    fn pointer(
        &mut self,
        _id: TabId,
        _x: i32,
        _y: i32,
        _action: PointerAction,
    ) -> Result<(), BrowserError> {
        Ok(())
    }
    fn scroll(
        &mut self,
        _id: TabId,
        _x: i32,
        _y: i32,
        _dx: i32,
        _dy: i32,
    ) -> Result<(), BrowserError> {
        Ok(())
    }
    fn poll(&mut self) -> EngineUpdates {
        EngineUpdates::default()
    }
}

/// Converts an address-bar input into a URL. A full parser is out of P0 scope.
pub fn normalize_address(input: &str) -> String {
    let value = input.trim();
    if value.is_empty() {
        return "about:blank".into();
    }
    if value.starts_with("https://") || value.starts_with("http://") || value.starts_with("about:")
    {
        value.into()
    } else if value.contains(' ') {
        format!(
            "https://www.google.com/search?q={}",
            value.split_whitespace().collect::<Vec<_>>().join("+")
        )
    } else {
        format!("https://{value}")
    }
}
