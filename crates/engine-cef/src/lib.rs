//! Reserved integration boundary for CEF. This is NOT a Chromium renderer yet.
//! Deliberately no fake BrowserEngine implementation: the core uses MockEngine
//! until a real CEF surface lifecycle is implemented and validated.
pub const CEF_UPSTREAM_REFERENCE: &str = "https://github.com/tauri-apps/cef-rs";
