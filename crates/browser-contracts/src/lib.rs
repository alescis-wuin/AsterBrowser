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
    Cancel,
}

/// Input descriptions are independent of the frontend toolkit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserKey {
    Text(String),
    Backspace,
    Tab,
    Enter,
    Escape,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Release,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub key: BrowserKey,
    pub action: KeyAction,
    pub modifiers: InputModifiers,
    pub repeat: bool,
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
        button: Option<PointerButton>,
        modifiers: InputModifiers,
    },
    Scroll {
        tab: TabId,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        modifiers: InputModifiers,
    },
    Key {
        tab: TabId,
        input: KeyInput,
    },
    Focus {
        tab: TabId,
        focused: bool,
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

/// A rectangle in physical image pixels, with an origin at the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl DamageRect {
    pub fn area(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn clipped(self, width: u32, height: u32) -> Option<Self> {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let right = self.x.saturating_add(self.width).min(width);
        let bottom = self.y.saturating_add(self.height).min(height);
        (right > x && bottom > y).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

/// One tightly packed, row-major BGRA8 rectangle (no stride padding).
#[derive(Clone, Debug)]
pub struct FramePatch {
    pub rect: DamageRect,
    pub bgra: Vec<u8>,
}

/// Full replaces all pixels; Patches requires a previously delivered Full
/// frame of the same size. Independent of either Slint or CEF.
#[derive(Debug)]
pub enum FramePayload {
    /// Shared immutable BGRA8 snapshot; no extra full-frame copy at dispatch.
    Full(std::sync::Arc<Vec<u8>>),
    Patches(Vec<FramePatch>),
}

#[derive(Debug)]
pub struct PageFrame {
    pub tab: TabId,
    pub width: u32,
    pub height: u32,
    pub payload: FramePayload,
}
impl PageFrame {
    pub fn transfer_bytes(&self) -> usize {
        match &self.payload {
            FramePayload::Full(bytes) => bytes.len(),
            FramePayload::Patches(patches) => patches.iter().map(|p| p.bgra.len()).sum(),
        }
    }
}

#[derive(Default, Debug)]
pub struct EngineUpdates {
    pub events: Vec<BrowserEvent>,
    /// Exactly one coherent accumulated update per tab per poll.
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
        _button: Option<PointerButton>,
        _modifiers: InputModifiers,
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
        _modifiers: InputModifiers,
    ) -> Result<(), BrowserError> {
        Ok(())
    }
    fn key(&mut self, _id: TabId, _input: &KeyInput) -> Result<(), BrowserError> {
        Ok(())
    }
    fn focus(&mut self, _id: TabId, _focused: bool) -> Result<(), BrowserError> {
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

pub mod render_options;
pub use render_options::{BufferCount, RenderMode, RenderOptions};
