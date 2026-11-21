//! CEF OSR adapter. All calls to CEF are made on the Slint UI thread.
//! The caller must run `CefRuntime::pump` regularly and call `shutdown` after
//! every `OnBeforeClose` acknowledgment is received.

use browser_contracts::{
    BrowserEngine, BrowserError, BrowserEvent, EngineUpdates, PageFrame, PointerAction, TabId,
};
use cef::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

const DEFAULT_WIDTH: u32 = 1000;
const DEFAULT_HEIGHT: u32 = 640;
const MAX_FRAME_PIXELS: u32 = 4096 * 2160;

wrap_app! {
    struct AsterApp { _marker: bool, }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefStringUtf16>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(command_line) = command_line {
                // Do not create an additional Chromium window outside of Slint.
                command_line.append_switch(Some(&"no-startup-window".into()));
            }
        }
    }
}

pub enum CefStartup {
    /// Child processes return their CEF-provided exit status without showing Slint.
    SubprocessExit(i32),
    Browser(CefRuntime),
}

pub struct CefRuntime {
    initialized: bool,
}

impl CefRuntime {
    pub fn start() -> Result<CefStartup, BrowserError> {
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let args = args::Args::new();
        let mut app = AsterApp::new(true);
        let subprocess_status = execute_process(
            Some(args.as_main_args()),
            Some(&mut app),
            std::ptr::null_mut(),
        );
        if subprocess_status >= 0 {
            return Ok(CefStartup::SubprocessExit(subprocess_status));
        }
        let settings = Settings {
            windowless_rendering_enabled: 1,
            external_message_pump: 1,
            // Do not weaken Chromium security just to make a prototype launch.
            no_sandbox: 0,
            ..Default::default()
        };
        if initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        ) != 1
        {
            return Err(BrowserError::Engine("CEF initialization failed".into()));
        }
        Ok(CefStartup::Browser(Self { initialized: true }))
    }

    /// Must be invoked on the thread that called `initialize`.
    pub fn pump() {
        do_message_loop_work();
    }

    /// CEF must not be shut down while any browser is still closing.
    pub fn shutdown(mut self, pending: &CloseTracker) -> Result<(), BrowserError> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while pending.remaining() > 0 && Instant::now() < deadline {
            Self::pump();
            thread::sleep(Duration::from_millis(8));
        }
        if pending.remaining() != 0 {
            // Intentionally skip cef::shutdown on this error path, rather than
            // shutdown native CEF while browser callbacks are still pending.
            self.initialized = false;
            return Err(BrowserError::Engine(format!(
                "{} Chromium browser(s) did not acknowledge close within 15s",
                pending.remaining()
            )));
        }
        cef::shutdown();
        self.initialized = false;
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct CloseTracker(Rc<Cell<usize>>);
impl CloseTracker {
    pub fn remaining(&self) -> usize {
        self.0.get()
    }
    fn begin(&self) {
        self.0.set(self.0.get() + 1);
    }
    fn acknowledged(&self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[derive(Default)]
struct Mailbox {
    events: Vec<BrowserEvent>,
    frames: HashMap<TabId, PageFrame>,
}

wrap_render_handler! {
    struct AsterRenderHandler {
        tab: TabId,
        size: Rc<Cell<(u32, u32)>>,
        mailbox: Rc<RefCell<Mailbox>>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let (width, height) = self.size.get();
                rect.x = 0;
                rect.y = 0;
                rect.width = width.max(1) as i32;
                rect.height = height.max(1) as i32;
            }
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            paint_type: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: i32,
            height: i32,
        ) {
            if paint_type != PaintElementType::default() || buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let (width, height) = (width as u32, height as u32);
            let Some(pixels) = width.checked_mul(height) else { return };
            if pixels > MAX_FRAME_PIXELS { return; }
            let Some(bytes) = pixels.checked_mul(4) else { return };
            // SAFETY: CEF documents the BGRA buffer as valid during OnPaint.
            // Copy it immediately; no native pointer escapes this callback.
            let bgra = unsafe { std::slice::from_raw_parts(buffer, bytes as usize) }.to_vec();
            self.mailbox.borrow_mut().frames.insert(self.tab, PageFrame {
                tab: self.tab, width, height, bgra,
            });
        }
    }
}

wrap_display_handler! {
    struct AsterDisplayHandler {
        tab: TabId,
        mailbox: Rc<RefCell<Mailbox>>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            if let Some(title) = title {
                self.mailbox.borrow_mut().events.push(BrowserEvent::TitleChanged {
                    tab: self.tab, title: title.to_string(),
                });
            }
        }
        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if frame.is_some_and(|frame| frame.is_main() != 0) {
                if let Some(url) = url {
                    self.mailbox.borrow_mut().events.push(BrowserEvent::UrlChanged {
                        tab: self.tab, url: url.to_string(),
                    });
                }
            }
        }
    }
}

wrap_load_handler! {
    struct AsterLoadHandler {
        tab: TabId,
        mailbox: Rc<RefCell<Mailbox>>,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut Browser>,
            loading: i32,
            _can_go_back: i32,
            _can_go_forward: i32,
        ) {
            self.mailbox.borrow_mut().events.push(BrowserEvent::LoadingChanged {
                tab: self.tab, loading: loading != 0,
            });
        }
    }
}

wrap_life_span_handler! {
    struct AsterLifeSpanHandler {
        tab: TabId,
        mailbox: Rc<RefCell<Mailbox>>,
        pending: CloseTracker,
    }

    impl LifeSpanHandler {
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            self.pending.acknowledged();
            let mut mailbox = self.mailbox.borrow_mut();
            mailbox.frames.remove(&self.tab);
            mailbox.events.push(BrowserEvent::Closed { tab: self.tab });
        }
    }
}

wrap_client! {
    struct AsterClient {
        render_handler: RenderHandler,
        display_handler: DisplayHandler,
        load_handler: LoadHandler,
        life_span_handler: LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(self.render_handler.clone()) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(self.display_handler.clone()) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(self.load_handler.clone()) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(self.life_span_handler.clone()) }
    }
}

struct CefTab {
    browser: Browser,
    size: Rc<Cell<(u32, u32)>>,
}

pub struct CefEngine {
    tabs: HashMap<TabId, CefTab>,
    mailbox: Rc<RefCell<Mailbox>>,
    pending: CloseTracker,
}

impl Default for CefEngine {
    fn default() -> Self {
        Self {
            tabs: HashMap::new(),
            mailbox: Rc::default(),
            pending: CloseTracker::default(),
        }
    }
}

impl CefEngine {
    pub fn close_tracker(&self) -> CloseTracker {
        self.pending.clone()
    }

    fn tab(&self, id: TabId) -> Result<&CefTab, BrowserError> {
        self.tabs.get(&id).ok_or(BrowserError::TabNotFound(id))
    }
}

impl BrowserEngine for CefEngine {
    fn open(&mut self, id: TabId, url: &str) -> Result<(), BrowserError> {
        if self.tabs.contains_key(&id) {
            return Err(BrowserError::Engine(format!(
                "duplicate browser tab {}",
                id.0
            )));
        }
        let size = Rc::new(Cell::new((DEFAULT_WIDTH, DEFAULT_HEIGHT)));
        let mut client = AsterClient::new(
            AsterRenderHandler::new(id, size.clone(), self.mailbox.clone()),
            AsterDisplayHandler::new(id, self.mailbox.clone()),
            AsterLoadHandler::new(id, self.mailbox.clone()),
            AsterLifeSpanHandler::new(id, self.mailbox.clone(), self.pending.clone()),
        );
        // An OSR surface is composited into Slint, not into a native child window.
        let window = WindowInfo {
            windowless_rendering_enabled: 1,
            ..Default::default()
        };
        let settings = BrowserSettings {
            windowless_frame_rate: 30,
            ..Default::default()
        };
        let browser = browser_host_create_browser_sync(
            Some(&window),
            Some(&mut client),
            Some(&CefString::from(url)),
            Some(&settings),
            None,
            None,
        )
        .ok_or_else(|| BrowserError::Engine("CEF browser creation failed".into()))?;
        self.tabs.insert(id, CefTab { browser, size });
        Ok(())
    }

    fn close(&mut self, id: TabId) -> Result<(), BrowserError> {
        let tab = self.tabs.remove(&id).ok_or(BrowserError::TabNotFound(id))?;
        if let Some(host) = tab.browser.host() {
            self.pending.begin();
            // Forced close intentionally avoids leaving P0 blocked on JS beforeunload dialogs.
            host.close_browser(1);
        }
        self.mailbox.borrow_mut().frames.remove(&id);
        Ok(())
    }

    fn navigate(&mut self, id: TabId, url: &str) -> Result<(), BrowserError> {
        let browser = &self.tab(id)?.browser;
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&CefString::from(url)));
        }
        Ok(())
    }
    fn back(&mut self, id: TabId) -> Result<(), BrowserError> {
        self.tab(id)?.browser.go_back();
        Ok(())
    }
    fn forward(&mut self, id: TabId) -> Result<(), BrowserError> {
        self.tab(id)?.browser.go_forward();
        Ok(())
    }
    fn reload(&mut self, id: TabId) -> Result<(), BrowserError> {
        self.tab(id)?.browser.reload();
        Ok(())
    }
    fn activate(&mut self, id: TabId) -> Result<(), BrowserError> {
        self.tab(id)?;
        for (&tab_id, tab) in &self.tabs {
            if let Some(host) = tab.browser.host() {
                let active = tab_id == id;
                host.was_hidden((!active) as i32);
                host.set_focus(active as i32);
                if active {
                    host.invalidate(PaintElementType::default());
                }
            }
        }
        Ok(())
    }
    fn resize(&mut self, id: TabId, width: u32, height: u32) -> Result<(), BrowserError> {
        let tab = self.tab(id)?;
        let size = (width.clamp(1, 4096), height.clamp(1, 2160));
        if tab.size.get() != size {
            tab.size.set(size);
            if let Some(host) = tab.browser.host() {
                host.was_resized();
            }
        }
        Ok(())
    }
    fn pointer(
        &mut self,
        id: TabId,
        x: i32,
        y: i32,
        action: PointerAction,
    ) -> Result<(), BrowserError> {
        let tab = self.tab(id)?;
        if let Some(host) = tab.browser.host() {
            let event = MouseEvent {
                x,
                y,
                ..Default::default()
            };
            match action {
                PointerAction::Down => {
                    host.set_focus(1);
                    host.send_mouse_click_event(Some(&event), MouseButtonType::default(), 0, 1);
                }
                PointerAction::Up => {
                    host.send_mouse_click_event(Some(&event), MouseButtonType::default(), 1, 1);
                }
                PointerAction::Move => {
                    host.send_mouse_move_event(Some(&event), 0);
                }
            }
        }
        Ok(())
    }
    fn scroll(&mut self, id: TabId, x: i32, y: i32, dx: i32, dy: i32) -> Result<(), BrowserError> {
        let tab = self.tab(id)?;
        if let Some(host) = tab.browser.host() {
            let event = MouseEvent {
                x,
                y,
                ..Default::default()
            };
            host.send_mouse_wheel_event(Some(&event), dx, dy);
        }
        Ok(())
    }
    fn poll(&mut self) -> EngineUpdates {
        let mut mailbox = self.mailbox.borrow_mut();
        EngineUpdates {
            events: std::mem::take(&mut mailbox.events),
            frames: mailbox.frames.drain().map(|(_, frame)| frame).collect(),
        }
    }
}
