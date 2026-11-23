//! Composition root. Core/contracts do not depend on the UI toolkit or CEF.
#[cfg(feature = "benchmark-control")]
mod benchmark_adapter;
mod workspace_session;
#[cfg(target_os = "linux")]
mod fontconfig;
#[cfg(target_os = "linux")]
mod linux_display;
#[cfg(not(feature = "cef"))]
use browser_contracts::BrowserSnapshot;
use browser_contracts::{
    BrowserCommand, BrowserEngine, LinuxDisplayMode, PumpMode, RenderMode, RenderOptions,
    WaylandGpuMode,
};
use browser_core::Browser;
#[cfg(not(feature = "cef"))]
use browser_core::MockEngine;
use std::{cell::RefCell, rc::Rc};
use workspace_session::WorkspaceSession;

fn dispatch_for<E: BrowserEngine + 'static>(
    browser: &Rc<RefCell<Browser<E>>>,
    workspaces: &Rc<RefCell<WorkspaceSession>>,
) -> frontend_slint::Dispatcher {
    let browser = Rc::clone(browser);
    let workspaces = Rc::clone(workspaces);
    Rc::new(move |command| {
        let mut browser = browser.borrow_mut();
        if let Err(error) = workspaces.borrow_mut().dispatch_browser(&mut browser, command) {
            eprintln!("browser command failed: {error}");
        }
        browser.snapshot()
    })
}

fn workspace_bridge<E: BrowserEngine + 'static>(
    browser: &Rc<RefCell<Browser<E>>>,
    workspaces: &Rc<RefCell<WorkspaceSession>>,
) -> frontend_slint::WorkspacesIntegration {
    let initial = workspaces.borrow().projection();
    let dispatch_browser = Rc::clone(browser);
    let dispatch_store = Rc::clone(workspaces);
    let projection_store = Rc::clone(workspaces);
    let revision_store = Rc::clone(workspaces);
    frontend_slint::WorkspacesIntegration {
        initial,
        dispatch: Rc::new(move |command| {
            let mut browser = dispatch_browser.borrow_mut();
            let mut workspaces = dispatch_store.borrow_mut();
            workspaces.dispatch_workspace(&mut browser, command);
            (browser.snapshot(), workspaces.projection())
        }),
        snapshot: Rc::new(move || projection_store.borrow().projection()),
        revision: Rc::new(move || revision_store.borrow().revision()),
    }
}

fn input_for<E: BrowserEngine + 'static>(
    browser: &Rc<RefCell<Browser<E>>>,
) -> frontend_slint::InputDispatcher {
    let browser = Rc::clone(browser);
    Rc::new(move |command| {
        if let Err(error) = browser.borrow_mut().dispatch(command) {
            eprintln!("browser input failed: {error}");
        }
    })
}

#[cfg(not(feature = "cef"))]
fn run_mock() -> Result<(), Box<dyn std::error::Error>> {
    let options = RenderOptions::from_env_and_args(std::env::args().skip(1))?;
    let mut browser = Browser::new(MockEngine::default());
    browser.dispatch(BrowserCommand::NewTab {
        url: "about:blank".into(),
    })?;
    let initial: BrowserSnapshot = browser.snapshot();
    let mut workspaces = WorkspaceSession::load();
    workspaces.attach_initial(&initial);
    let shared = Rc::new(RefCell::new(browser));
    let workspaces = Rc::new(RefCell::new(workspaces));
    let dispatcher = dispatch_for(&shared, &workspaces);
    let input_dispatcher = input_for(&shared);
    let poller: frontend_slint::Poller = {
        let shared = Rc::clone(&shared);
        let workspaces = Rc::clone(&workspaces);
        Rc::new(move || {
            let tick = shared.borrow_mut().tick();
            workspaces.borrow_mut().reconcile(&tick.snapshot);
            tick
        })
    };
    frontend_slint::run(
        initial,
        dispatcher,
        input_dispatcher,
        poller,
        "Mock engine",
        frontend_slint::FrontendConfig::cpu(options.buffers.for_cpu())
            .with_workspaces(workspace_bridge(&shared, &workspaces)),
    )?;
    shared.borrow_mut().close_all()?;
    Ok(())
}

#[cfg(feature = "cef")]
fn run_cef() -> Result<(), Box<dyn std::error::Error>> {
    use engine_cef::{CefEngine, CefRuntime, CefStartup};

    // Subprocess execution must happen before parsing Aster's own arguments or
    // selecting a UI backend. Chromium renderer processes never create Slint.
    // Read only the render mode before CEF launches. All other validation still
    // happens after execute_process so Chromium subprocesses can exit normally.
    // Unknown Chromium subprocess flags are ignored by RenderOptions.
    let early_options = RenderOptions::from_env_and_args(std::env::args().skip(1));
    #[cfg(feature = "cef-gpu")]
    let gpu_osr_requested = early_options.as_ref().is_ok_and(|options| {
        if options.mode == RenderMode::Cpu {
            return false;
        }
        // In auto mode, allow a native Wayland CPU-first profile that does
        // not waste eight seconds waiting for a known failing capture path.
        let native_wayland = options.linux_display == LinuxDisplayMode::Wayland
            || (options.linux_display == LinuxDisplayMode::Auto
                && std::env::var_os("DISPLAY").is_none()
                && std::env::var_os("WAYLAND_DISPLAY").is_some());
        !(options.mode == RenderMode::Auto
            && native_wayland
            && options.wayland_gpu == WaylandGpuMode::Cpu)
    });
    #[cfg(not(feature = "cef-gpu"))]
    let gpu_osr_requested = false;
    // CEF subprocesses only execute_process; display validation belongs to
    // the browser process and runs before any Chromium threads are created.
    #[cfg(target_os = "linux")]
    if !std::env::args().any(|arg| arg.starts_with("--type=")) {
        if let Ok(options) = &early_options {
            linux_display::preflight(options.linux_display)?;
        }
    }
    let display = early_options
        .as_ref()
        .map_or(Default::default(), |o| o.linux_display);
    let pump_options = early_options
        .as_ref()
        .map_or(Default::default(), |o| o.pump);
    let cef_angle = early_options
        .as_ref()
        .map_or(Default::default(), |o| o.cef_angle);
    let runtime = match CefRuntime::start_with_options(
        gpu_osr_requested,
        display,
        cef_angle,
        pump_options,
    )? {
        CefStartup::SubprocessExit(code) => std::process::exit(code),
        CefStartup::Browser(runtime) => runtime,
    };
    let options = match RenderOptions::from_env_and_args(std::env::args().skip(1)) {
        Ok(value) => value,
        Err(error) => {
            runtime.shutdown(&engine_cef::CloseTracker::default())?;
            return Err(error.into());
        }
    };
    let mut engine = CefEngine::default();
    #[cfg(feature = "cef-gpu")]
    engine.set_gpu_first_frame_timeout(std::time::Duration::from_millis(
        options.gpu_first_frame_timeout_ms,
    ));
    let mut frontend = frontend_slint::FrontendConfig::cpu(options.buffers.for_cpu());
    #[cfg(feature = "cef-gpu")]
    if gpu_osr_requested {
        let hub = browser_render_wgpu::GpuHub::with_options(options.buffers, options.gpu);
        match frontend_slint::select_wgpu_renderer() {
            Ok(()) => {
                eprintln!(
                    "[aster] experimental CEF GPU: buffers={} damage={:?} threshold={}pct lazy_init={} sync={:?} import_filter={:?}",
                    options.buffers.get(),
                    options.gpu.damage,
                    options.gpu.threshold_pct,
                    options.gpu.lazy_init,
                    options.gpu.sync,
                    options.gpu.import_filter
                );
                engine.enable_gpu(hub.clone());
                frontend = frontend.with_gpu(hub).with_gpu_frame_notifications(
                    options.gpu_presentation == browser_contracts::GpuPresentationMode::Notify,
                );
            }
            Err(error) if options.mode == RenderMode::Auto => {
                eprintln!("[aster] WGPU unavailable: {error}; falling back to CPU renderer");
            }
            Err(error) => {
                runtime.shutdown(&engine.close_tracker())?;
                return Err(Box::new(error));
            }
        }
    }
    #[cfg(not(feature = "cef-gpu"))]
    if options.mode == RenderMode::Gpu {
        runtime.shutdown(&engine.close_tracker())?;
        return Err("GPU renderer requires cargo --features cef-gpu".into());
    }
    if options.mode == RenderMode::Cpu || !gpu_osr_requested {
        eprintln!(
            "[aster] using CPU OSR with {} RGB buffer(s); wayland_gpu={:?}",
            options.buffers.for_cpu().get(),
            options.wayland_gpu
        );
    }
    let pending = engine.close_tracker();
    let mut browser = Browser::new(engine);
    // Benchmarks never request an unrelated public page before their local
    // fixture loads. Ordinary launches retain the original welcome page.
    #[cfg(feature = "benchmark-control")]
    let initial_url = if std::env::var_os("ASTER_BENCH_PORT").is_some() {
        "about:blank"
    } else {
        "https://example.org"
    };
    #[cfg(not(feature = "benchmark-control"))]
    let initial_url = "https://example.org";
    browser.dispatch(BrowserCommand::NewTab {
        url: initial_url.into(),
    })?;
    let initial = browser.snapshot();
    let mut workspaces = WorkspaceSession::load();
    workspaces.attach_initial(&initial);
    let shared = Rc::new(RefCell::new(browser));
    let workspaces = Rc::new(RefCell::new(workspaces));
    let dispatcher = dispatch_for(&shared, &workspaces);
    frontend = frontend.with_workspaces(workspace_bridge(&shared, &workspaces));
    let input_dispatcher = input_for(&shared);
    let poller: frontend_slint::Poller = {
        let shared = Rc::clone(&shared);
        let workspaces = Rc::clone(&workspaces);
        let pump = runtime.pump_handle();
        Rc::new(move || {
            pump.pump_if_due();
            let tick = shared.borrow_mut().tick();
            workspaces.borrow_mut().reconcile(&tick.snapshot);
            tick
        })
    };
    if options.pump.mode == PumpMode::Event {
        let signal = frontend_slint::EventLoopSignal::default();
        let pump = runtime.pump_handle();
        pump.set_waker(signal.notifier());
        // A single-shot timer tracks CEF's next deadline; CEF's callback
        // only wakes the Slint event loop and never executes CEF off-thread.
        frontend = frontend.with_event_pump(signal, Rc::new(move || pump.next_delay()));
        eprintln!(
            "[aster:scheduler] event_pump=enabled idle_heartbeat_ms={} wake_policy={:?} gpu_presentation={:?}",
            options.pump.idle_ms, options.pump.wake, options.gpu_presentation
        );
    }

    if std::env::var("ASTER_LIFECYCLE").ok().as_deref() == Some("1") {
        eprintln!("[aster:lifecycle] frontend_enter");
    }
    #[cfg(feature = "benchmark-control")]
    let (frontend, _benchmark_server) = match benchmark_adapter::install(&shared, frontend) {
        Ok(result) => result,
        Err(error) => {
            let _ = shared.borrow_mut().close_all();
            runtime.shutdown(&pending)?;
            return Err(error);
        }
    };
    let ui_result = frontend_slint::run(
        initial,
        dispatcher,
        input_dispatcher,
        poller,
        "Chromium / CEF",
        frontend,
    );
    if std::env::var("ASTER_LIFECYCLE").ok().as_deref() == Some("1") {
        eprintln!(
            "[aster:lifecycle] frontend_exit ui_ok={} starting_browser_close",
            ui_result.is_ok()
        );
    }
    // Close all native browsers and wait for CEF acknowledgments before shutdown.
    let close_result = shared.borrow_mut().close_all();
    drop(shared);
    close_result?;
    runtime.shutdown(&pending)?;
    ui_result?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    fontconfig::configure()?; // Must run before CEF or Slint initializes threads.
    #[cfg(feature = "cef")]
    {
        return run_cef();
    }
    #[cfg(not(feature = "cef"))]
    {
        run_mock()
    }
}
