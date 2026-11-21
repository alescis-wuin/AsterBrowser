//! CEF OSR adapter. All calls to CEF are made on the Slint UI thread.
//! The caller must run `CefPumpHandle::pump_if_due` regularly and call `shutdown` after
//! every `OnBeforeClose` acknowledgment is received.

use browser_contracts::{
    BrowserEngine, BrowserError, BrowserEvent, BrowserKey, DamageRect, EngineUpdates, FramePayload,
    InputModifiers, KeyAction, KeyInput, PointerAction, PointerButton, TabId,
};
use cef::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    thread,
    time::{Duration, Instant},
};

use crate::damage::DamageSurface;
use crate::pump::{PumpController, PumpReason, PumpRequests};

const MAX_PUMP_INTERVAL_MS: u64 = 33;

fn pump_max_interval() -> Duration {
    let ms = std::env::var("ASTER_PUMP_MAX_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|ms| (8..=33).contains(ms))
        .unwrap_or(MAX_PUMP_INTERVAL_MS);
    Duration::from_millis(ms)
}

/// Copy safe, clipped physical damage rectangles from CEF's signed coordinates.
fn paint_damage(rects: Option<&[Rect]>, width: u32, height: u32) -> Vec<DamageRect> {
    rects
        .unwrap_or(&[])
        .iter()
        .filter_map(|r| {
            let left = i64::from(r.x).clamp(0, i64::from(width));
            let top = i64::from(r.y).clamp(0, i64::from(height));
            let right = (i64::from(r.x) + i64::from(r.width)).clamp(0, i64::from(width));
            let bottom = (i64::from(r.y) + i64::from(r.height)).clamp(0, i64::from(height));
            if right <= left || bottom <= top {
                return None;
            }
            Some(DamageRect {
                x: left as u32,
                y: top as u32,
                width: (right - left) as u32,
                height: (bottom - top) as u32,
            })
        })
        .collect()
}

const DEFAULT_WIDTH: u32 = 1000;
const DEFAULT_HEIGHT: u32 = 640;
const MAX_FRAME_PIXELS: u32 = 4096 * 2160;

/// CEF OSR supports a configurable paint rate. Invalid values use the safe default.
fn osr_frame_rate(value: Option<&str>) -> i32 {
    value
        .and_then(|text| text.parse::<i32>().ok())
        .filter(|fps| (1..=60).contains(fps))
        .unwrap_or(60)
}

fn cef_flags(mods: InputModifiers, button: Option<PointerButton>, repeat: bool) -> u32 {
    use sys::cef_event_flags_t as Flags;
    let mut value = 0;
    if mods.shift {
        value |= Flags::EVENTFLAG_SHIFT_DOWN.0;
    }
    if mods.control {
        value |= Flags::EVENTFLAG_CONTROL_DOWN.0;
    }
    if mods.alt {
        value |= Flags::EVENTFLAG_ALT_DOWN.0;
    }
    if mods.meta {
        value |= Flags::EVENTFLAG_COMMAND_DOWN.0;
    }
    if repeat {
        value |= Flags::EVENTFLAG_IS_REPEAT.0;
    }
    value |= match button {
        Some(PointerButton::Left) => Flags::EVENTFLAG_LEFT_MOUSE_BUTTON.0,
        Some(PointerButton::Right) => Flags::EVENTFLAG_RIGHT_MOUSE_BUTTON.0,
        Some(PointerButton::Middle) => Flags::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0,
        None => 0,
    };
    value
}

fn mouse_button(button: PointerButton) -> MouseButtonType {
    match button {
        PointerButton::Left => sys::cef_mouse_button_type_t::MBT_LEFT.into(),
        PointerButton::Middle => sys::cef_mouse_button_type_t::MBT_MIDDLE.into(),
        PointerButton::Right => sys::cef_mouse_button_type_t::MBT_RIGHT.into(),
    }
}

fn virtual_key(key: &BrowserKey) -> i32 {
    match key {
        BrowserKey::Backspace => 0x08,
        BrowserKey::Tab => 0x09,
        BrowserKey::Enter => 0x0d,
        BrowserKey::Escape => 0x1b,
        BrowserKey::Delete => 0x2e,
        BrowserKey::Left => 0x25,
        BrowserKey::Right => 0x27,
        BrowserKey::Up => 0x26,
        BrowserKey::Down => 0x28,
        BrowserKey::Home => 0x24,
        BrowserKey::End => 0x23,
        BrowserKey::PageUp => 0x21,
        BrowserKey::PageDown => 0x22,
        BrowserKey::Insert => 0x2d,
        BrowserKey::F(n) if (1..=24).contains(n) => 0x70 + i32::from(*n) - 1,
        BrowserKey::F(_) => 0,
        BrowserKey::Text(text) => match text.chars().next() {
            Some(ch) if ch.is_ascii_alphabetic() => ch.to_ascii_uppercase() as i32,
            Some(ch) if ch.is_ascii_digit() => ch as i32,
            Some(' ') => 0x20,
            Some(';') | Some(':') => 0xba,
            Some('=') | Some('+') => 0xbb,
            Some(',') | Some('<') => 0xbc,
            Some('-') | Some('_') => 0xbd,
            Some('.') | Some('>') => 0xbe,
            Some('/') | Some('?') => 0xbf,
            Some('`') | Some('~') => 0xc0,
            Some('[') | Some('{') => 0xdb,
            Some('\\') | Some('|') => 0xdc,
            Some(']') | Some('}') => 0xdd,
            Some('\'') | Some('"') => 0xde,
            _ => 0,
        },
    }
}

fn dispatch_key_event(
    host: &mut BrowserHost,
    kind: sys::cef_key_event_type_t,
    vk: i32,
    character: u16,
    flags: u32,
) {
    let event = KeyEvent {
        type_: kind.into(),
        windows_key_code: vk,
        // Slint exposes logical keys, not native XKB scan codes. Keep this
        // explicitly unknown until a native input backend is available.
        native_key_code: 0,
        modifiers: flags,
        character,
        unmodified_character: character,
        ..Default::default()
    };
    host.send_key_event(Some(&event));
}

wrap_browser_process_handler! {
    struct AsterProcessHandler { requests: PumpRequests, }

    impl BrowserProcessHandler {
        // CEF may call this method from any thread. Do not call any UI API here.
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            self.requests.schedule(delay_ms);
        }
    }
}

wrap_app! {
    struct AsterApp { requests: PumpRequests, }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(AsterProcessHandler::new(self.requests.clone()))
        }

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
    pump: CefPumpHandle,
}

/// GUI-independent handle for scheduling CEF calls on the thread that initialized it.
#[derive(Clone)]
pub struct CefPumpHandle(Rc<PumpController>);

impl CefPumpHandle {
    pub fn pump_if_due(&self) {
        if let Some(reason) = self.0.due(Instant::now()) {
            self.run(reason);
        }
    }
    fn run(&self, reason: PumpReason) {
        self.0.begin();
        let started = Instant::now();
        do_message_loop_work();
        self.0.end(reason, started.elapsed());
    }
    fn force_pump(&self) {
        // During shutdown, callbacks must continue even without schedule requests.
        self.run(PumpReason::Fallback);
    }
}

impl CefRuntime {
    pub fn start() -> Result<CefStartup, BrowserError> {
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let args = args::Args::new();
        let requests = PumpRequests::default();
        let mut app = AsterApp::new(requests.clone());
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
        let perf = std::env::var("ASTER_PERF").ok().as_deref() == Some("1");
        let pump = CefPumpHandle(Rc::new(PumpController::new(
            requests,
            pump_max_interval(),
            perf,
        )));
        Ok(CefStartup::Browser(Self {
            initialized: true,
            pump,
        }))
    }

    pub fn pump_handle(&self) -> CefPumpHandle {
        self.pump.clone()
    }

    /// CEF must not be shut down while any browser is still closing.
    pub fn shutdown(mut self, pending: &CloseTracker) -> Result<(), BrowserError> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while pending.remaining() > 0 && Instant::now() < deadline {
            self.pump.force_pump();
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
struct PaintStats {
    started: Option<Instant>,
    produced: u64,
    unchanged_frames: u64,
    coalesced: u64,
    dirty_pixels: u64,
    frame_pixels: u64,
    bytes_compared: u64,
    bytes_copied: u64,
    bytes_emitted: u64,
    full_updates: u64,
    partial_updates: u64,
    render_skipped: u64,
    cpu_time: Duration,
    cpu_max: Duration,
}

#[derive(Default)]
struct Mailbox {
    events: Vec<BrowserEvent>,
    surfaces: HashMap<TabId, DamageSurface>,
    stats: PaintStats,
}

wrap_render_handler! {
    struct AsterRenderHandler {
        tab: TabId,
        size: Rc<Cell<(u32, u32)>>,
        mailbox: Rc<RefCell<Mailbox>>,
        perf_enabled: bool,
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
            dirty_rects: Option<&[Rect]>,
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
            // SAFETY: CEF owns this BGRA buffer only for the duration of OnPaint.
            // The accumulator compares/copies the changed regions synchronously.
            let bgra = unsafe { std::slice::from_raw_parts(buffer, bytes as usize) };
            let dirty = paint_damage(dirty_rects, width, height);
            let started = Instant::now();
            let mut mailbox = self.mailbox.borrow_mut();
            let Some(result) = mailbox.surfaces.entry(self.tab).or_default().paint(bgra, width, height, &dirty) else { return };
            if self.perf_enabled {
                let stats = &mut mailbox.stats;
                stats.started.get_or_insert_with(Instant::now);
                stats.produced += 1;
                stats.unchanged_frames += u64::from(result.unchanged);
                stats.render_skipped += u64::from(result.unchanged);
                stats.coalesced += u64::from(result.coalesced);
                stats.dirty_pixels += result.dirty_pixels.min(result.total_pixels);
                stats.frame_pixels += result.total_pixels;
                stats.bytes_compared += result.compared_bytes;
                stats.bytes_copied += result.copied_bytes;
                let cpu = started.elapsed();
                stats.cpu_time += cpu;
                stats.cpu_max = stats.cpu_max.max(cpu);
            }
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
            mailbox.surfaces.remove(&self.tab);
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
    perf_enabled: bool,
}

impl Default for CefEngine {
    fn default() -> Self {
        Self {
            tabs: HashMap::new(),
            mailbox: Rc::default(),
            pending: CloseTracker::default(),
            perf_enabled: std::env::var("ASTER_PERF").ok().as_deref() == Some("1"),
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
            AsterRenderHandler::new(id, size.clone(), self.mailbox.clone(), self.perf_enabled),
            AsterDisplayHandler::new(id, self.mailbox.clone()),
            AsterLoadHandler::new(id, self.mailbox.clone()),
            AsterLifeSpanHandler::new(id, self.mailbox.clone(), self.pending.clone()),
        );
        // An OSR surface is composited into Slint, not into a native child window.
        let window = WindowInfo {
            windowless_rendering_enabled: 1,
            ..Default::default()
        };
        let fps = osr_frame_rate(std::env::var("ASTER_OSR_FPS").ok().as_deref());
        let settings = BrowserSettings {
            windowless_frame_rate: fps,
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
        self.mailbox.borrow_mut().surfaces.remove(&id);
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
        button: Option<PointerButton>,
        modifiers: InputModifiers,
    ) -> Result<(), BrowserError> {
        let tab = self.tab(id)?;
        if let Some(host) = tab.browser.host() {
            let event = MouseEvent {
                x,
                y,
                modifiers: cef_flags(
                    modifiers,
                    if action == PointerAction::Up {
                        None
                    } else {
                        button
                    },
                    false,
                ),
                ..Default::default()
            };
            match action {
                PointerAction::Down => {
                    if let Some(button) = button {
                        host.set_focus(1);
                        host.send_mouse_click_event(Some(&event), mouse_button(button), 0, 1);
                    }
                }
                PointerAction::Up => {
                    if let Some(button) = button {
                        host.send_mouse_click_event(Some(&event), mouse_button(button), 1, 1);
                    }
                }
                PointerAction::Move => host.send_mouse_move_event(Some(&event), 0),
                PointerAction::Cancel => host.send_capture_lost_event(),
            }
        }
        Ok(())
    }
    fn scroll(
        &mut self,
        id: TabId,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        modifiers: InputModifiers,
    ) -> Result<(), BrowserError> {
        let tab = self.tab(id)?;
        if let Some(host) = tab.browser.host() {
            let event = MouseEvent {
                x,
                y,
                modifiers: cef_flags(modifiers, None, false),
                ..Default::default()
            };
            host.send_mouse_wheel_event(Some(&event), dx, dy);
        }
        Ok(())
    }
    fn focus(&mut self, id: TabId, focused: bool) -> Result<(), BrowserError> {
        if let Some(host) = self.tab(id)?.browser.host() {
            host.set_focus(focused as i32);
        }
        Ok(())
    }
    fn key(&mut self, id: TabId, input: &KeyInput) -> Result<(), BrowserError> {
        let Some(mut host) = self.tab(id)?.browser.host() else {
            return Ok(());
        };
        let vk = virtual_key(&input.key);
        let flags = cef_flags(input.modifiers, None, input.repeat);
        match input.action {
            KeyAction::Press => {
                if vk != 0 {
                    dispatch_key_event(
                        &mut host,
                        sys::cef_key_event_type_t::KEYEVENT_RAWKEYDOWN,
                        vk,
                        0,
                        flags,
                    );
                }
                // CEF requires CHAR events for text insertion, distinct from keydown.
                // Do not insert characters while a keyboard shortcut is held.
                if !input.modifiers.control && !input.modifiers.alt && !input.modifiers.meta {
                    let text = match &input.key {
                        BrowserKey::Text(text) => Some(text.as_str()),
                        BrowserKey::Enter => Some("\r"),
                        _ => None,
                    };
                    if let Some(text) = text {
                        for unit in text.encode_utf16() {
                            dispatch_key_event(
                                &mut host,
                                sys::cef_key_event_type_t::KEYEVENT_CHAR,
                                vk,
                                unit,
                                flags,
                            );
                        }
                    }
                }
            }
            KeyAction::Release => {
                if vk != 0 {
                    dispatch_key_event(
                        &mut host,
                        sys::cef_key_event_type_t::KEYEVENT_KEYUP,
                        vk,
                        0,
                        flags,
                    );
                }
            }
        }
        Ok(())
    }
    fn poll(&mut self) -> EngineUpdates {
        let mut mailbox = self.mailbox.borrow_mut();
        let frames: Vec<_> = mailbox
            .surfaces
            .iter_mut()
            .filter_map(|(&tab, surface)| surface.take_frame(tab))
            .collect();
        if self.perf_enabled {
            for frame in &frames {
                match &frame.payload {
                    FramePayload::Full(_) => mailbox.stats.full_updates += 1,
                    FramePayload::Patches(_) => mailbox.stats.partial_updates += 1,
                }
                mailbox.stats.bytes_emitted += frame.transfer_bytes() as u64;
            }
            if let Some(started) = mailbox.stats.started {
                let elapsed = started.elapsed();
                if elapsed >= Duration::from_secs(2) {
                    let stats = &mailbox.stats;
                    let count = stats.produced.max(1);
                    eprintln!(
                        "[aster:cef] paint={:.1}/s unchanged_frames={} presentation_skipped={} coalesced={} dirty_area_pct={:.1} copied={:.1}MiB/s emitted={:.1}MiB/s compared={:.1}MiB/s full_updates={} partial_updates={} paint_cpu_avg={:.2}ms paint_cpu_max={:.2}ms",
                        stats.produced as f64 / elapsed.as_secs_f64(),
                        stats.unchanged_frames,
                        stats.render_skipped,
                        stats.coalesced,
                        100.0 * stats.dirty_pixels as f64 / stats.frame_pixels.max(1) as f64,
                        stats.bytes_copied as f64 / 1048576.0 / elapsed.as_secs_f64(),
                        stats.bytes_emitted as f64 / 1048576.0 / elapsed.as_secs_f64(),
                        stats.bytes_compared as f64 / 1048576.0 / elapsed.as_secs_f64(),
                        stats.full_updates,
                        stats.partial_updates,
                        stats.cpu_time.as_secs_f64() * 1000.0 / count as f64,
                        stats.cpu_max.as_secs_f64() * 1000.0,
                    );
                    mailbox.stats = PaintStats::default();
                }
            }
        }
        EngineUpdates {
            events: std::mem::take(&mut mailbox.events),
            frames,
        }
    }
}

#[cfg(test)]
mod frame_rate_tests {
    use super::osr_frame_rate;

    #[test]
    fn validates_frame_rate_override() {
        assert_eq!(osr_frame_rate(None), 60);
        assert_eq!(osr_frame_rate(Some("30")), 30);
        assert_eq!(osr_frame_rate(Some("60")), 60);
        assert_eq!(osr_frame_rate(Some("0")), 60);
        assert_eq!(osr_frame_rate(Some("61")), 60);
        assert_eq!(osr_frame_rate(Some("invalid")), 60);
    }
}

#[cfg(test)]
mod damage_rect_tests {
    use super::*;
    #[test]
    fn negative_cef_rectangles_are_clipped_correctly() {
        let raw = [Rect {
            x: -4,
            y: -1,
            width: 7,
            height: 5,
        }];
        assert_eq!(
            paint_damage(Some(&raw), 10, 10),
            vec![DamageRect {
                x: 0,
                y: 0,
                width: 3,
                height: 4
            }]
        );
    }
}
