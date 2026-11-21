//! Composition root. Core/contracts do not depend on the UI toolkit or CEF.
#[cfg(not(feature = "cef"))]
use browser_contracts::BrowserSnapshot;
use browser_contracts::{BrowserCommand, BrowserEngine, RenderMode, RenderOptions};
use browser_core::Browser;
#[cfg(not(feature = "cef"))]
use browser_core::MockEngine;
use std::{cell::RefCell, rc::Rc};

fn dispatch_for<E: BrowserEngine + 'static>(
    browser: &Rc<RefCell<Browser<E>>>,
) -> frontend_slint::Dispatcher {
    let browser = Rc::clone(browser);
    Rc::new(move |command| {
        let mut browser = browser.borrow_mut();
        if let Err(error) = browser.dispatch(command) {
            eprintln!("browser command failed: {error}");
        }
        browser.snapshot()
    })
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
    let shared = Rc::new(RefCell::new(browser));
    let dispatcher = dispatch_for(&shared);
    let input_dispatcher = input_for(&shared);
    let poller: frontend_slint::Poller = {
        let shared = Rc::clone(&shared);
        Rc::new(move || shared.borrow_mut().tick())
    };
    frontend_slint::run(
        initial,
        dispatcher,
        input_dispatcher,
        poller,
        "Mock engine",
        frontend_slint::FrontendConfig::cpu(options.buffers),
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
    #[cfg(feature = "cef-gpu")]
    let gpu_osr_requested = RenderOptions::from_env_and_args(std::env::args().skip(1))
        .is_ok_and(|options| options.mode != RenderMode::Cpu);
    #[cfg(not(feature = "cef-gpu"))]
    let gpu_osr_requested = false;
    let runtime = match CefRuntime::start(gpu_osr_requested)? {
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
    let mut frontend = frontend_slint::FrontendConfig::cpu(options.buffers);
    #[cfg(feature = "cef-gpu")]
    if options.mode != RenderMode::Cpu {
        let hub = browser_render_wgpu::GpuHub::new(options.buffers);
        match frontend_slint::select_wgpu_renderer() {
            Ok(()) => {
                eprintln!(
                    "[aster] using experimental CEF shared-texture GPU rendering with {} presentation buffer(s)",
                    options.buffers.get()
                );
                engine.enable_gpu(hub.clone());
                frontend = frontend.with_gpu(hub);
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
    if options.mode == RenderMode::Cpu {
        eprintln!(
            "[aster] using CPU OSR with {} RGB buffer(s)",
            options.buffers.get()
        );
    }
    let pending = engine.close_tracker();
    let mut browser = Browser::new(engine);
    browser.dispatch(BrowserCommand::NewTab {
        url: "https://example.org".into(),
    })?;
    let initial = browser.snapshot();
    let shared = Rc::new(RefCell::new(browser));
    let dispatcher = dispatch_for(&shared);
    let input_dispatcher = input_for(&shared);
    let poller: frontend_slint::Poller = {
        let shared = Rc::clone(&shared);
        let pump = runtime.pump_handle();
        Rc::new(move || {
            pump.pump_if_due();
            shared.borrow_mut().tick()
        })
    };

    let ui_result = frontend_slint::run(
        initial,
        dispatcher,
        input_dispatcher,
        poller,
        "Chromium / CEF",
        frontend,
    );
    // Close all native browsers and wait for CEF acknowledgments before shutdown.
    let close_result = shared.borrow_mut().close_all();
    drop(shared);
    close_result?;
    runtime.shutdown(&pending)?;
    ui_result?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "cef")]
    {
        return run_cef();
    }
    #[cfg(not(feature = "cef"))]
    {
        run_mock()
    }
}
