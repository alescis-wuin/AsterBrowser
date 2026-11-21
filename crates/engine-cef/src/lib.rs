//! The only crate allowed to reference the Chromium Embedded Framework API.
//! `cef-runtime` is deliberately opt-in so the mock frontend builds without CEF.

#[cfg(feature = "cef-runtime")]
mod damage;
#[cfg(feature = "cef-runtime")]
mod pump;
#[cfg(feature = "cef-runtime")]
mod runtime;

#[cfg(feature = "cef-runtime")]
pub use runtime::{CefEngine, CefPumpHandle, CefRuntime, CefStartup, CloseTracker};
