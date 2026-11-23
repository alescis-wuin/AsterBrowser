//! Slint frontend. No direct dependency on Chromium or browser-core.
mod frame_surface;
use browser_contracts::{
    BrowserCommand, BrowserKey, BrowserSnapshot, BrowserTick, BufferCount, InputModifiers,
    KeyAction, KeyInput, PointerAction, PointerButton, TabId,
};
use frame_surface::{BufferedRgbSurface, UpdateKind};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
slint::include_modules!();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SignalKind {
    #[default]
    Pump,
    Frame,
}

/// CEF notifications can arrive from any Chromium thread; this handle only
/// queues a Slint event-loop callback and coalesces redundant wakeups.
#[derive(Clone, Default)]
pub struct EventLoopSignal(Arc<EventWakeState>);
#[derive(Default)]
struct EventWakeState {
    kind: SignalKind,
    pending: AtomicBool,
    requested: AtomicU64,
    posted: AtomicU64,
    dispatched: AtomicU64,
    coalesced: AtomicU64,
}
#[derive(Clone, Copy, Default)]
struct WakeCounters {
    requested: u64,
    posted: u64,
    dispatched: u64,
    coalesced: u64,
}

thread_local! {
    // UI-thread-only destination. This is not a global CEF UI callback and is
    // cleared on every Slint run exit, including error paths.
    static EVENT_PUMP_CALLBACK: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
    static EVENT_FRAME_CALLBACK: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
    static EXTERNAL_CONTROL_CALLBACK: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None);
}

impl EventLoopSignal {
    pub fn for_frames() -> Self {
        Self(Arc::new(EventWakeState {
            kind: SignalKind::Frame,
            ..Default::default()
        }))
    }
    pub fn notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let signal = self.clone();
        Arc::new(move || signal.notify())
    }
    pub fn notify(&self) {
        self.0.requested.fetch_add(1, Ordering::Relaxed);
        if self.0.pending.swap(true, Ordering::AcqRel) {
            self.0.coalesced.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let state = self.clone();
        if slint::invoke_from_event_loop(move || {
            state.0.pending.store(false, Ordering::Release);
            state.0.dispatched.fetch_add(1, Ordering::Relaxed);
            match state.0.kind {
                SignalKind::Pump => EVENT_PUMP_CALLBACK.with(|slot| {
                    if let Some(callback) = slot.borrow().as_ref() {
                        callback();
                    }
                }),
                SignalKind::Frame => EVENT_FRAME_CALLBACK.with(|slot| {
                    if let Some(callback) = slot.borrow().as_ref() {
                        callback();
                    }
                }),
            }
        })
        .is_ok()
        {
            self.0.posted.fetch_add(1, Ordering::Relaxed);
        } else {
            // Before the UI event loop starts and after it exits, a bounded
            // timer remains responsible for missed notifications.
            self.0.pending.store(false, Ordering::Release);
        }
    }
    fn counters(&self) -> WakeCounters {
        WakeCounters {
            requested: self.0.requested.load(Ordering::Relaxed),
            posted: self.0.posted.load(Ordering::Relaxed),
            dispatched: self.0.dispatched.load(Ordering::Relaxed),
            coalesced: self.0.coalesced.load(Ordering::Relaxed),
        }
    }
}

/// Optional adapter for test-only commands dispatched on the actual Slint UI thread.
/// The benchmark transport lives in a separate crate; this frontend knows only
/// about window actions returned by a generic callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalUiAction {
    Resize { width: u32, height: u32 },
    Input { tab: TabId },
    Quit,
}

/// Safe to call from a socket/worker thread. No UI state crosses threads.
pub fn notify_external_commands() {
    let _ = slint::invoke_from_event_loop(|| {
        EXTERNAL_CONTROL_CALLBACK.with(|slot| {
            let callback = slot.borrow().clone();
            if let Some(callback) = callback {
                callback();
            }
        });
    });
}

#[derive(Clone)]
pub struct EventPumpConfig {
    pub signal: EventLoopSignal,
    pub next_delay: Rc<dyn Fn() -> Duration>,
}

pub struct FrontendConfig {
    pub buffers: BufferCount,
    pub event_pump: Option<EventPumpConfig>,
    pub external_control: Option<Rc<dyn Fn() -> Vec<ExternalUiAction>>>,
    #[cfg(feature = "gpu")]
    pub gpu_frame_notifications: bool,
    #[cfg(feature = "gpu")]
    pub gpu: Option<browser_render_wgpu::GpuHub>,
}
impl FrontendConfig {
    pub fn cpu(buffers: BufferCount) -> Self {
        Self {
            buffers,
            event_pump: None,
            external_control: None,
            #[cfg(feature = "gpu")]
            gpu_frame_notifications: false,
            #[cfg(feature = "gpu")]
            gpu: None,
        }
    }
    pub fn with_external_control(mut self, handler: Rc<dyn Fn() -> Vec<ExternalUiAction>>) -> Self {
        self.external_control = Some(handler);
        self
    }
    pub fn with_event_pump(
        mut self,
        signal: EventLoopSignal,
        next_delay: Rc<dyn Fn() -> Duration>,
    ) -> Self {
        self.event_pump = Some(EventPumpConfig { signal, next_delay });
        self
    }
    #[cfg(feature = "gpu")]
    pub fn with_gpu_frame_notifications(mut self, enabled: bool) -> Self {
        self.gpu_frame_notifications = enabled;
        self
    }
    #[cfg(feature = "gpu")]
    pub fn with_gpu(mut self, hub: browser_render_wgpu::GpuHub) -> Self {
        self.gpu = Some(hub);
        self
    }
}

/// Must be called before the first Slint window is created.
#[cfg(feature = "gpu")]
pub fn select_wgpu_renderer() -> Result<(), slint::PlatformError> {
    use slint::wgpu_30::{WGPUConfiguration, WGPUSettings, wgpu};
    let mut settings = WGPUSettings::default();
    settings.backends = wgpu::Backends::VULKAN;
    if std::env::var("ASTER_GPU_POWER").ok().as_deref() == Some("low") {
        settings.power_preference = wgpu::PowerPreference::LowPower;
    } else {
        settings.power_preference = wgpu::PowerPreference::HighPerformance;
    }
    // The CEF DMA-BUF importer requires this device feature. Do not silently
    // select a different GPU or backend if the shared-import path is unavailable.
    #[cfg(target_os = "linux")]
    {
        settings.device_required_features |= wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
    }
    slint::BackendSelector::new()
        .require_wgpu_30(WGPUConfiguration::Automatic(settings))
        .select()
}

#[cfg(feature = "gpu")]
fn install_gpu_context(ui: &BrowserWindow, hub: browser_render_wgpu::GpuHub) {
    let cloned = hub.clone();
    let result = ui
        .window()
        .set_rendering_notifier(move |state, graphics_api| match state {
            slint::RenderingState::RenderingSetup => {
                if let slint::GraphicsAPI::WGPU30 { device, queue, .. } = graphics_api {
                    cloned.set_device(device.clone(), queue.clone());
                } else {
                    cloned.fail("Slint did not provide a WGPU30 renderer");
                }
            }
            slint::RenderingState::RenderingTeardown => cloned.clear(),
            _ => {}
        });
    if let Err(error) = result {
        hub.fail(format!("Slint WGPU rendering notifier failed: {error}"));
    }
}

#[cfg(feature = "gpu")]
fn present_gpu_image(
    ui: &BrowserWindow,
    hub: &browser_render_wgpu::GpuHub,
    active: Option<TabId>,
    last_shown: &mut Option<(TabId, u64)>,
) -> Option<Duration> {
    let Some(tab) = active else {
        *last_shown = None;
        return None;
    };
    let Some(revision) = hub.revision(tab) else {
        *last_shown = None;
        return None;
    };
    if *last_shown == Some((tab, revision)) {
        return None;
    }
    let Some(presented) = hub.current(tab) else {
        return None;
    };
    match slint::Image::try_from(presented.texture) {
        Ok(image) => {
            ui.set_page_image(image);
            ui.set_has_page(true);
            // With a single buffer the texture identity is stable. Request a
            // new composition even if the image property's identity is reused.
            ui.window().request_redraw();
            *last_shown = Some((tab, presented.revision));
            Some(presented.submitted_at.elapsed())
        }
        Err(error) => {
            hub.fail(format!("Cannot display WGPU frame in Slint: {error:?}"));
            *last_shown = None;
            None
        }
    }
}

/// Measured from the most recent input dispatch to the next *UI image update*.
/// This is an observational proxy, not verified input-to-scanout latency.
#[derive(Default)]
struct UiLatency {
    last_input: HashMap<TabId, Instant>,
    input_to_ui_ms: Vec<f64>,
    gpu_submit_to_ui_ms: Vec<f64>,
}
impl UiLatency {
    fn record_input(&mut self, id: TabId) {
        self.last_input.insert(id, Instant::now());
    }
    fn record_frame(&mut self, id: TabId, gpu_submit_to_ui: Option<Duration>) {
        if let Some(at) = self.last_input.remove(&id) {
            if self.input_to_ui_ms.len() < 8192 {
                self.input_to_ui_ms
                    .push(at.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if let Some(delay) = gpu_submit_to_ui {
            if self.gpu_submit_to_ui_ms.len() < 8192 {
                self.gpu_submit_to_ui_ms.push(delay.as_secs_f64() * 1000.0);
            }
        }
    }
    fn percentiles(samples: &[f64]) -> (f64, f64, f64) {
        if samples.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut values = samples.to_vec();
        values.sort_by(f64::total_cmp);
        let at = |p: f64| values[((values.len() - 1) as f64 * p).ceil() as usize];
        (at(0.50), at(0.95), at(0.99))
    }
    fn report(&mut self) {
        let (a, b, c) = Self::percentiles(&self.input_to_ui_ms);
        let (d, e, f) = Self::percentiles(&self.gpu_submit_to_ui_ms);
        eprintln!(
            "[aster:latency] input_to_ui_samples={} input_to_ui_p50_ms={:.2} input_to_ui_p95_ms={:.2} input_to_ui_p99_ms={:.2} gpu_submit_to_ui_samples={} gpu_submit_to_ui_p50_ms={:.2} gpu_submit_to_ui_p95_ms={:.2} gpu_submit_to_ui_p99_ms={:.2}",
            self.input_to_ui_ms.len(),
            a,
            b,
            c,
            self.gpu_submit_to_ui_ms.len(),
            d,
            e,
            f
        );
        self.input_to_ui_ms.clear();
        self.gpu_submit_to_ui_ms.clear();
    }
}

#[cfg(target_os = "linux")]
fn process_cpu_time() -> Option<Duration> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime only fills the local timespec. Process CPU includes
    // Aster's threads but excludes separate Chromium subprocesses.
    if unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) } != 0 {
        return None;
    }
    Some(
        Duration::from_secs(u64::try_from(time.tv_sec).ok()?)
            + Duration::from_nanos(u64::try_from(time.tv_nsec).ok()?),
    )
}
#[cfg(not(target_os = "linux"))]
fn process_cpu_time() -> Option<Duration> {
    None
}

pub type Dispatcher = Rc<dyn Fn(BrowserCommand) -> BrowserSnapshot>;
/// Dispatches high-frequency input without cloning tab state.
pub type InputDispatcher = Rc<dyn Fn(BrowserCommand)>;
pub type Poller = Rc<dyn Fn() -> BrowserTick>;

/// Translate Slint logical keys at the frontend boundary, never in browser-core.
fn convert_key(text: &str) -> Option<BrowserKey> {
    use slint::platform::Key as K;
    let specials = [
        (K::Backspace, BrowserKey::Backspace),
        (K::Tab, BrowserKey::Tab),
        (K::Backtab, BrowserKey::Tab),
        (K::Return, BrowserKey::Enter),
        (K::Escape, BrowserKey::Escape),
        (K::Delete, BrowserKey::Delete),
        (K::LeftArrow, BrowserKey::Left),
        (K::RightArrow, BrowserKey::Right),
        (K::UpArrow, BrowserKey::Up),
        (K::DownArrow, BrowserKey::Down),
        (K::Home, BrowserKey::Home),
        (K::End, BrowserKey::End),
        (K::PageUp, BrowserKey::PageUp),
        (K::PageDown, BrowserKey::PageDown),
        (K::Insert, BrowserKey::Insert),
        (K::F1, BrowserKey::F(1)),
        (K::F2, BrowserKey::F(2)),
        (K::F3, BrowserKey::F(3)),
        (K::F4, BrowserKey::F(4)),
        (K::F5, BrowserKey::F(5)),
        (K::F6, BrowserKey::F(6)),
        (K::F7, BrowserKey::F(7)),
        (K::F8, BrowserKey::F(8)),
        (K::F9, BrowserKey::F(9)),
        (K::F10, BrowserKey::F(10)),
        (K::F11, BrowserKey::F(11)),
        (K::F12, BrowserKey::F(12)),
    ];
    if let Some((_, key)) = specials
        .into_iter()
        .find(|(slint_key, _)| text == char::from(*slint_key).to_string())
    {
        return Some(key);
    }
    if !text.is_empty()
        && text
            .chars()
            .all(|ch| !ch.is_control() && !(0xe000..=0xf8ff).contains(&(ch as u32)))
    {
        return Some(BrowserKey::Text(text.to_owned()));
    }
    None
}

fn modifiers(shift: bool, control: bool, alt: bool, meta: bool) -> InputModifiers {
    InputModifiers {
        shift,
        control,
        alt,
        meta,
    }
}

fn render(
    ui: &BrowserWindow,
    snapshot: &BrowserSnapshot,
    frames: &mut HashMap<TabId, BufferedRgbSurface>,
    shown_tab: &Cell<Option<TabId>>,
    engine_name: &str,
) {
    let tabs: Vec<TabView> = snapshot
        .tabs
        .iter()
        .map(|tab| TabView {
            id: tab.id.0 as i32,
            title: SharedString::from(tab.title.as_str()),
            active: snapshot.active == Some(tab.id),
        })
        .collect();
    ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
    ui.set_active_tab_id(snapshot.active.map_or(-1, |tab| tab.0 as i32));
    let current = snapshot
        .tabs
        .iter()
        .find(|tab| Some(tab.id) == snapshot.active);
    ui.set_current_url(current.map(|tab| tab.url.as_str()).unwrap_or("").into());
    ui.set_status_text(format!("{} — {} tab(s)", engine_name, snapshot.tabs.len()).into());
    // Metadata changes must not cause the entire webpage Image to be re-presented.
    if shown_tab.get() != snapshot.active {
        shown_tab.set(snapshot.active);
        if let Some(surface) = snapshot.active.and_then(|tab| frames.get(&tab)) {
            ui.set_page_image(surface.image());
            ui.set_has_page(true);
        } else {
            ui.set_has_page(false);
        }
    }
    frames.retain(|id, _| snapshot.tabs.iter().any(|tab| tab.id == *id));
}

fn apply_command(
    command: BrowserCommand,
    ui: &slint::Weak<BrowserWindow>,
    state: &Rc<RefCell<BrowserSnapshot>>,
    frames: &Rc<RefCell<HashMap<TabId, BufferedRgbSurface>>>,
    shown_tab: &Rc<Cell<Option<TabId>>>,
    dispatcher: &Dispatcher,
    engine_name: &str,
) {
    let snapshot = dispatcher(command);
    let changed = *state.borrow() != snapshot;
    if changed {
        *state.borrow_mut() = snapshot;
        if let Some(ui) = ui.upgrade() {
            render(
                &ui,
                &state.borrow(),
                &mut frames.borrow_mut(),
                shown_tab,
                engine_name,
            );
        }
    }
}

/// Re-arm a single-shot timer to the next CEF deadline or idle heartbeat.
/// Its callback only captures a Weak timer reference, avoiding an Rc cycle.
fn arm_event_timer(
    timer: &Rc<slint::Timer>,
    tick: &Rc<RefCell<Box<dyn FnMut()>>>,
    next_delay: &Rc<dyn Fn() -> Duration>,
    active: &Rc<Cell<bool>>,
) {
    let delay = (next_delay)().clamp(Duration::from_millis(1), Duration::from_millis(1000));
    let weak_timer = Rc::downgrade(timer);
    let tick = Rc::clone(tick);
    let next_delay = Rc::clone(next_delay);
    let active = Rc::clone(active);
    timer.start(slint::TimerMode::SingleShot, delay, move || {
        if !active.get() {
            return;
        }
        if let Ok(mut callback) = tick.try_borrow_mut() {
            (*callback)();
        }
        if let Some(timer) = weak_timer.upgrade() {
            arm_event_timer(&timer, &tick, &next_delay, &active);
        }
    });
}

pub fn run(
    initial: BrowserSnapshot,
    dispatcher: Dispatcher,
    input_dispatcher: InputDispatcher,
    poller: Poller,
    engine_name: &'static str,
    config: FrontendConfig,
) -> Result<(), slint::PlatformError> {
    let ui = BrowserWindow::new()?;
    #[cfg(feature = "gpu")]
    let gpu = config.gpu.clone();
    #[cfg(feature = "gpu")]
    if let Some(hub) = gpu.clone() {
        install_gpu_context(&ui, hub);
    }
    #[cfg(feature = "gpu")]
    let last_gpu_image: Rc<RefCell<Option<(TabId, u64)>>> = Rc::default();
    let state = Rc::new(RefCell::new(initial));
    let frames: Rc<RefCell<HashMap<TabId, BufferedRgbSurface>>> = Rc::default();
    let shown_tab = Rc::new(Cell::new(None));
    let latency = Rc::new(RefCell::new(UiLatency::default()));
    render(
        &ui,
        &state.borrow(),
        &mut frames.borrow_mut(),
        &shown_tab,
        engine_name,
    );

    macro_rules! command_handler {
        ($name:ident, $make:expr) => {{
            let weak = ui.as_weak();
            let state = state.clone();
            let frames = frames.clone();
            let shown_tab = shown_tab.clone();
            let dispatcher = dispatcher.clone();
            ui.$name(move || {
                apply_command(
                    $make,
                    &weak,
                    &state,
                    &frames,
                    &shown_tab,
                    &dispatcher,
                    engine_name,
                )
            });
        }};
    }
    command_handler!(
        on_add_tab,
        BrowserCommand::NewTab {
            url: "about:blank".into()
        }
    );
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let shown_tab = shown_tab.clone();
        let dispatcher = dispatcher.clone();
        ui.on_close_tab(move |id| {
            apply_command(
                BrowserCommand::CloseTab(TabId(id as u64)),
                &weak,
                &state,
                &frames,
                &shown_tab,
                &dispatcher,
                engine_name,
            )
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let shown_tab = shown_tab.clone();
        let dispatcher = dispatcher.clone();
        ui.on_select_tab(move |id| {
            apply_command(
                BrowserCommand::ActivateTab(TabId(id as u64)),
                &weak,
                &state,
                &frames,
                &shown_tab,
                &dispatcher,
                engine_name,
            )
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let shown_tab = shown_tab.clone();
        let dispatcher = dispatcher.clone();
        ui.on_navigate(move |text| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Navigate {
                        tab,
                        input: text.to_string(),
                    },
                    &weak,
                    &state,
                    &frames,
                    &shown_tab,
                    &dispatcher,
                    engine_name,
                );
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let shown_tab = shown_tab.clone();
        let dispatcher = dispatcher.clone();
        ui.on_back(move || {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Back(tab),
                    &weak,
                    &state,
                    &frames,
                    &shown_tab,
                    &dispatcher,
                    engine_name,
                );
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let shown_tab = shown_tab.clone();
        let dispatcher = dispatcher.clone();
        ui.on_forward(move || {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Forward(tab),
                    &weak,
                    &state,
                    &frames,
                    &shown_tab,
                    &dispatcher,
                    engine_name,
                );
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let shown_tab = shown_tab.clone();
        let dispatcher = dispatcher.clone();
        ui.on_reload(move || {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Reload(tab),
                    &weak,
                    &state,
                    &frames,
                    &shown_tab,
                    &dispatcher,
                    engine_name,
                );
            }
        });
    }
    {
        let state = state.clone();
        let input_dispatcher = input_dispatcher.clone();
        let latency = latency.clone();
        ui.on_page_pointer(move |x, y, action, button, shift, control, alt, meta| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                let action = match action {
                    0 => PointerAction::Down,
                    1 => PointerAction::Up,
                    2 => PointerAction::Move,
                    _ => PointerAction::Cancel,
                };
                let button = match button {
                    1 => Some(PointerButton::Left),
                    2 => Some(PointerButton::Right),
                    3 => Some(PointerButton::Middle),
                    _ => None,
                };
                if matches!(action, PointerAction::Down) {
                    latency.borrow_mut().record_input(tab);
                }
                input_dispatcher(BrowserCommand::Pointer {
                    tab,
                    x,
                    y,
                    action,
                    button,
                    modifiers: modifiers(shift, control, alt, meta),
                });
            }
        });
    }
    {
        let state = state.clone();
        let input_dispatcher = input_dispatcher.clone();
        let latency = latency.clone();
        ui.on_page_scroll(move |x, y, dx, dy, shift, control, alt, meta| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                latency.borrow_mut().record_input(tab);
                input_dispatcher(BrowserCommand::Scroll {
                    tab,
                    x,
                    y,
                    dx,
                    dy,
                    modifiers: modifiers(shift, control, alt, meta),
                });
            }
        });
    }
    {
        let state = state.clone();
        let input_dispatcher = input_dispatcher.clone();
        let latency = latency.clone();
        ui.on_page_key(move |text, action, shift, control, alt, meta, repeat| {
            let active = state.borrow().active;
            if let (Some(tab), Some(key)) = (active, convert_key(text.as_str())) {
                if action == 0 {
                    latency.borrow_mut().record_input(tab);
                }
                input_dispatcher(BrowserCommand::Key {
                    tab,
                    input: KeyInput {
                        key,
                        action: if action == 0 {
                            KeyAction::Press
                        } else {
                            KeyAction::Release
                        },
                        modifiers: modifiers(shift, control, alt, meta),
                        repeat,
                    },
                });
            }
        });
    }
    {
        let state = state.clone();
        let input_dispatcher = input_dispatcher.clone();
        ui.on_page_focus(move |focused| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                input_dispatcher(BrowserCommand::Focus { tab, focused });
            }
        });
    }

    // An external message pump is driven by the native Slint event loop.
    // No cross-thread CEF callbacks and no nested event loop.
    let weak = ui.as_weak();
    let mut last_viewport: Option<(TabId, i32, i32)> = None;
    let pump_ms = std::env::var("ASTER_PUMP_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|ms| (4..=33).contains(ms))
        .unwrap_or(8);
    let perf_enabled = std::env::var("ASTER_PERF").ok().as_deref() == Some("1");
    let mut stats_since = Instant::now();
    let mut ticks = 0_u64;
    let mut presented = 0_u64;
    let mut partial_presented = 0_u64;
    let mut full_presented = 0_u64;
    let mut background_updated = 0_u64;
    let mut presentation_skipped = 0_u64;
    let mut converted_pixels = 0_u64;
    let mut converted_updates = 0_u64;
    let mut pump_total = Duration::ZERO;
    let mut pump_max = Duration::ZERO;
    let mut convert_total = Duration::ZERO;
    let mut convert_max = Duration::ZERO;
    let event_pump = config.event_pump.clone();
    #[cfg(feature = "gpu")]
    let on_demand = config.gpu_frame_notifications && gpu.is_some();
    #[cfg(feature = "gpu")]
    let frame_callback_state = state.clone();
    #[cfg(feature = "gpu")]
    let tick_gpu_hub = gpu.clone();
    #[cfg(feature = "gpu")]
    let tick_gpu_image = last_gpu_image.clone();
    let tick_latency = latency.clone();
    let mut cpu_since = process_cpu_time();
    let signal_for_stats = event_pump.as_ref().map(|p| p.signal.clone());
    let mut previous_wakes = WakeCounters::default();
    let tick: Rc<RefCell<Box<dyn FnMut()>>> = Rc::new(RefCell::new(Box::new(move || {
        let began = Instant::now();
        let update = poller();
        let pump_duration = began.elapsed();
        if perf_enabled {
            ticks += 1;
            pump_total += pump_duration;
            pump_max = pump_max.max(pump_duration);
        }
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let mut image_cache = frames.borrow_mut();
        let active = update.snapshot.active;
        let mut new_active_frame = false;
        for frame in update.frames {
            let id = frame.tab;
            // Ignore obsolete callbacks after the tab has been closed.
            if !update.snapshot.tabs.iter().any(|tab| tab.id == id) {
                continue;
            }
            let started = Instant::now();
            let applied = if let Some(surface) = image_cache.get_mut(&id) {
                // A resize changes both dimensions; only a new Full frame is valid.
                if surface.matches_size(&frame) {
                    surface.apply(frame)
                } else {
                    BufferedRgbSurface::from_frame(frame, config.buffers).map(|(new, applied)| {
                        *surface = new;
                        applied
                    })
                }
            } else {
                BufferedRgbSurface::from_frame(frame, config.buffers).map(|(surface, applied)| {
                    image_cache.insert(id, surface);
                    applied
                })
            };
            if let Some(applied) = applied {
                if Some(id) == active {
                    new_active_frame = true;
                    if perf_enabled {
                        presented += 1;
                        match applied.kind {
                            UpdateKind::Full => full_presented += 1,
                            UpdateKind::Partial => partial_presented += 1,
                        }
                    }
                } else if perf_enabled {
                    background_updated += 1;
                }
                if perf_enabled {
                    converted_pixels += applied.converted_pixels;
                    converted_updates += 1;
                }
            } else if perf_enabled {
                presentation_skipped += 1;
            }
            if perf_enabled {
                let duration = started.elapsed();
                convert_total += duration;
                convert_max = convert_max.max(duration);
            }
        }
        let changed = *state.borrow() != update.snapshot;
        let previously_shown = shown_tab.get();
        if changed {
            *state.borrow_mut() = update.snapshot;
            render(
                &ui,
                &state.borrow(),
                &mut image_cache,
                &shown_tab,
                engine_name,
            );
        }
        // Update the Image only when the active surface actually changed.
        if new_active_frame
            && active == state.borrow().active
            && (!changed || previously_shown == active)
        {
            if let Some(surface) = active.and_then(|id| image_cache.get(&id)) {
                ui.set_page_image(surface.image());
                ui.set_has_page(true);
                shown_tab.set(active);
                tick_latency
                    .borrow_mut()
                    .record_frame(active.unwrap(), None);
            }
        }
        drop(image_cache);
        #[cfg(feature = "gpu")]
        if let Some(hub) = tick_gpu_hub.as_ref() {
            let tab = state.borrow().active;
            let active_changed = tick_gpu_image.borrow().as_ref().map(|(id, _)| *id) != tab;
            if !on_demand || active_changed {
                if let Some(delay) =
                    present_gpu_image(&ui, hub, tab, &mut tick_gpu_image.borrow_mut())
                {
                    if let Some(id) = tab {
                        tick_latency.borrow_mut().record_frame(id, Some(delay));
                    }
                }
            }
            hub.report_if_due();
        }
        let active = state.borrow().active;
        if let Some(tab) = active {
            let (width, height) = (ui.get_viewport_width(), ui.get_viewport_height());
            let next_viewport = (tab, width, height);
            if width > 0 && height > 0 && last_viewport != Some(next_viewport) {
                dispatcher(BrowserCommand::Resize {
                    tab,
                    width: width as u32,
                    height: height as u32,
                });
                last_viewport = Some(next_viewport);
            }
        } else {
            last_viewport = None;
        }
        if perf_enabled && stats_since.elapsed() >= Duration::from_secs(2) {
            let elapsed = stats_since.elapsed().as_secs_f64();
            eprintln!(
                "[aster:slint] ticks={:.1}/s frames_delivered={:.1}/s full_updates={} partial_updates={} background_updates={} presentation_skipped={} converted={:.1}Mpx/s pump_avg={:.2}ms pump_max={:.2}ms convert_avg={:.2}ms convert_max={:.2}ms",
                ticks as f64 / elapsed,
                presented as f64 / elapsed,
                full_presented,
                partial_presented,
                background_updated,
                presentation_skipped,
                converted_pixels as f64 / 1_000_000.0 / elapsed,
                pump_total.as_secs_f64() * 1000.0 / ticks.max(1) as f64,
                pump_max.as_secs_f64() * 1000.0,
                convert_total.as_secs_f64() * 1000.0 / converted_updates.max(1) as f64,
                convert_max.as_secs_f64() * 1000.0,
            );
            tick_latency.borrow_mut().report();
            if let Some(now) = process_cpu_time() {
                let delta_cpu = cpu_since.map_or(Duration::ZERO, |prev| now.saturating_sub(prev));
                eprintln!(
                    "[aster:load] process_cpu_pct={:.2} process_cpu_ms={:.2} window_ms={:.2}",
                    delta_cpu.as_secs_f64() / elapsed * 100.0,
                    delta_cpu.as_secs_f64() * 1000.0,
                    elapsed * 1000.0
                );
                cpu_since = Some(now);
            }
            if let Some(signal) = signal_for_stats.as_ref() {
                let current = signal.counters();
                eprintln!(
                    "[aster:scheduler] mode=event wake_requested={} wake_posted={} wake_dispatched={} wake_coalesced={} ticks_per_sec={:.1}",
                    current.requested.saturating_sub(previous_wakes.requested),
                    current.posted.saturating_sub(previous_wakes.posted),
                    current.dispatched.saturating_sub(previous_wakes.dispatched),
                    current.coalesced.saturating_sub(previous_wakes.coalesced),
                    ticks as f64 / elapsed,
                );
                previous_wakes = current;
            }
            stats_since = Instant::now();
            ticks = 0;
            presented = 0;
            full_presented = 0;
            partial_presented = 0;
            background_updated = 0;
            presentation_skipped = 0;
            converted_pixels = 0;
            converted_updates = 0;
            pump_total = Duration::ZERO;
            pump_max = Duration::ZERO;
            convert_total = Duration::ZERO;
            convert_max = Duration::ZERO;
        }
    })));
    #[cfg(feature = "gpu")]
    if on_demand {
        if let Some(hub) = gpu.as_ref() {
            let frame_signal = EventLoopSignal::for_frames();
            let callback_gpu = hub.clone();
            let callback_state = frame_callback_state.clone();
            let callback_weak = ui.as_weak();
            let callback_last = last_gpu_image.clone();
            let callback_latency = latency.clone();
            EVENT_FRAME_CALLBACK.with(|slot| {
                *slot.borrow_mut() = Some(Rc::new(move || {
                    let Some(ui) = callback_weak.upgrade() else {
                        return;
                    };
                    let active = callback_state.borrow().active;
                    if let Some(delay) = present_gpu_image(
                        &ui,
                        &callback_gpu,
                        active,
                        &mut callback_last.borrow_mut(),
                    ) {
                        if let Some(id) = active {
                            callback_latency.borrow_mut().record_frame(id, Some(delay));
                        }
                    }
                }));
            });
            hub.set_frame_notifier(Some(frame_signal.notifier()));
            eprintln!("[aster:presentation] gpu_frames=notify pump_independent=true");
        }
    }
    if let Some(handler) = config.external_control.as_ref() {
        let handler = Rc::clone(handler);
        let control_tick = Rc::clone(&tick);
        let control_window = ui.as_weak();
        let control_latency = Rc::clone(&latency);
        EXTERNAL_CONTROL_CALLBACK.with(|slot| {
            *slot.borrow_mut() = Some(Rc::new(move || {
                let actions = handler();
                let mut quitting = false;
                for action in actions {
                    match action {
                        ExternalUiAction::Resize { width, height } => {
                            if let Some(ui) = control_window.upgrade() {
                                ui.window()
                                    .set_size(slint::PhysicalSize::new(width, height));
                            }
                        }
                        ExternalUiAction::Input { tab } => {
                            control_latency.borrow_mut().record_input(tab);
                        }
                        ExternalUiAction::Quit => {
                            quitting = true;
                        }
                    }
                }
                if quitting {
                    let _ = slint::quit_event_loop();
                } else if let Ok(mut callback) = control_tick.try_borrow_mut() {
                    (*callback)();
                }
            }));
        });
    }
    let timer = Rc::new(slint::Timer::default());
    let active = Rc::new(Cell::new(true));
    if let Some(event) = event_pump.as_ref() {
        // Only the UI-thread callback owns widgets/CEF. Cross-thread wakeups
        // carry no non-Send frontend state through invoke_from_event_loop.
        let weak_timer = Rc::downgrade(&timer);
        let callback_tick = Rc::clone(&tick);
        let callback_next = Rc::clone(&event.next_delay);
        let callback_active = Rc::clone(&active);
        EVENT_PUMP_CALLBACK.with(|slot| {
            *slot.borrow_mut() = Some(Rc::new(move || {
                if !callback_active.get() {
                    return;
                }
                if let Ok(mut callback) = callback_tick.try_borrow_mut() {
                    (*callback)();
                }
                if let Some(timer) = weak_timer.upgrade() {
                    arm_event_timer(&timer, &callback_tick, &callback_next, &callback_active);
                }
            }));
        });
        arm_event_timer(&timer, &tick, &event.next_delay, &active);
        if std::env::var("ASTER_PERF").ok().as_deref() == Some("1") {
            eprintln!("[aster:scheduler] mode=event idle_heartbeat_enabled=1");
        }
    } else {
        let callback_tick = Rc::clone(&tick);
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(pump_ms),
            move || {
                if let Ok(mut callback) = callback_tick.try_borrow_mut() {
                    (*callback)();
                }
            },
        );
    }
    let ui_result = ui.run();
    active.set(false);
    EXTERNAL_CONTROL_CALLBACK.with(|slot| {
        slot.borrow_mut().take();
    });
    #[cfg(feature = "gpu")]
    if on_demand {
        if let Some(hub) = gpu.as_ref() {
            hub.set_frame_notifier(None);
        }
        EVENT_FRAME_CALLBACK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
    // Remove the UI-thread destination before closing CEF. Late notifications
    // cannot resurrect a destroyed Slint window.
    if event_pump.is_some() {
        EVENT_PUMP_CALLBACK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
    if std::env::var("ASTER_LIFECYCLE").ok().as_deref() == Some("1") {
        eprintln!(
            "[aster:lifecycle] slint_run_return ok={}",
            ui_result.is_ok()
        );
    }
    timer.stop();
    drop(timer);
    drop(tick);
    drop(ui);
    if std::env::var("ASTER_LIFECYCLE").ok().as_deref() == Some("1") {
        eprintln!("[aster:lifecycle] slint_window_dropped");
    }
    ui_result
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use slint::platform::Key;

    #[test]
    fn special_and_unicode_keys_are_normalized_without_cef() {
        assert_eq!(
            convert_key(&char::from(Key::Backspace).to_string()),
            Some(BrowserKey::Backspace)
        );
        assert_eq!(
            convert_key(&char::from(Key::LeftArrow).to_string()),
            Some(BrowserKey::Left)
        );
        assert_eq!(convert_key("é"), Some(BrowserKey::Text("é".into())));
        assert_eq!(convert_key(""), None);
    }
}

#[cfg(test)]
mod latency_tests {
    use super::UiLatency;
    #[test]
    fn percentiles_are_ordered_and_support_empty_windows() {
        assert_eq!(UiLatency::percentiles(&[]), (0.0, 0.0, 0.0));
        assert_eq!(
            UiLatency::percentiles(&(0..100).map(|x| x as f64).collect::<Vec<_>>()),
            (50.0, 95.0, 99.0)
        );
    }
}
