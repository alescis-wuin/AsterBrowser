use browser_contracts::{BrowserCommand, BrowserSnapshot};
use browser_core::{Browser, MockEngine};
use std::{cell::RefCell, rc::Rc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut browser = Browser::new(MockEngine::default());
    browser.dispatch(BrowserCommand::NewTab { url: "about:blank".into() })?;
    let initial: BrowserSnapshot = browser.snapshot();
    let shared = Rc::new(RefCell::new(browser));
    let dispatcher = Rc::new(move |command| {
        let mut browser = shared.borrow_mut();
        if let Err(error) = browser.dispatch(command) { eprintln!("browser command failed: {error}"); }
        browser.snapshot()
    });
    frontend_slint::run(initial, dispatcher)?;
    Ok(())
}
