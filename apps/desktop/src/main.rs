//! Composition root. Neither browser-core nor frontend-slint knows about CEF.
use browser_contracts::{BrowserCommand, BrowserEngine, BrowserSnapshot};
use browser_core::{Browser, MockEngine};
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

fn run_mock() -> Result<(), Box<dyn std::error::Error>> {
    let mut browser = Browser::new(MockEngine::default());
    browser.dispatch(BrowserCommand::NewTab {
        url: "about:blank".into(),
    })?;
    let initial: BrowserSnapshot = browser.snapshot();
    let shared = Rc::new(RefCell::new(browser));
    let dispatcher = dispatch_for(&shared);
    let poller: frontend_slint::Poller = {
        let shared = Rc::clone(&shared);
        Rc::new(move || shared.borrow_mut().tick())
    };
    frontend_slint::run(initial, dispatcher, poller, "Mock engine")?;
    shared.borrow_mut().close_all()?;
    Ok(())
}

#[cfg(feature = "cef")]
fn run_cef() -> Result<(), Box<dyn std::error::Error>> {
    use engine_cef::{CefEngine, CefRuntime, CefStartup};

    let runtime = match CefRuntime::start()? {
        CefStartup::SubprocessExit(code) => std::process::exit(code),
        CefStartup::Browser(runtime) => runtime,
    };
    let engine = CefEngine::default();
    let pending = engine.close_tracker();
    let mut browser = Browser::new(engine);
    browser.dispatch(BrowserCommand::NewTab {
        url: "https://example.org".into(),
    })?;
    let initial = browser.snapshot();
    let shared = Rc::new(RefCell::new(browser));
    let dispatcher = dispatch_for(&shared);
    let poller: frontend_slint::Poller = {
        let shared = Rc::clone(&shared);
        Rc::new(move || {
            CefRuntime::pump();
            shared.borrow_mut().tick()
        })
    };

    let ui_result = frontend_slint::run(initial, dispatcher, poller, "Chromium / CEF");
    // The renderer is allowed to finish its asynchronous OnBeforeClose callbacks
    // before the CEF library is shut down. This also runs after a UI error.
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
