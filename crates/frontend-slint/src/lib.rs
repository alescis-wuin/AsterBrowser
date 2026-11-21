//! Slint frontend. No direct dependency on Chromium or browser-core.
mod frame_surface;
use browser_contracts::{
    BrowserCommand, BrowserKey, BrowserSnapshot, BrowserTick, InputModifiers, KeyAction, KeyInput,
    PageFrame, PointerAction, PointerButton, TabId,
};
use frame_surface::{RgbSurface, UpdateKind};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};
slint::include_modules!();

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
    frames: &mut HashMap<TabId, RgbSurface>,
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
    frames: &Rc<RefCell<HashMap<TabId, RgbSurface>>>,
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

pub fn run(
    initial: BrowserSnapshot,
    dispatcher: Dispatcher,
    input_dispatcher: InputDispatcher,
    poller: Poller,
    engine_name: &'static str,
) -> Result<(), slint::PlatformError> {
    let ui = BrowserWindow::new()?;
    let state = Rc::new(RefCell::new(initial));
    let frames: Rc<RefCell<HashMap<TabId, RgbSurface>>> = Rc::default();
    let shown_tab = Rc::new(Cell::new(None));
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
        ui.on_page_scroll(move |x, y, dx, dy, shift, control, alt, meta| {
            let active = state.borrow().active;
            if let Some(tab) = active {
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
        ui.on_page_key(move |text, action, shift, control, alt, meta, repeat| {
            let active = state.borrow().active;
            if let (Some(tab), Some(key)) = (active, convert_key(text.as_str())) {
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
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(pump_ms), move || {
        let began = Instant::now();
        let update = poller();
        let pump_duration = began.elapsed();
        if perf_enabled {
            ticks += 1;
            pump_total += pump_duration;
            pump_max = pump_max.max(pump_duration);
        }
        let Some(ui) = weak.upgrade() else { return; };
        let mut image_cache = frames.borrow_mut();
        let active = update.snapshot.active;
        let mut new_active_frame = false;
        for frame in update.frames {
            let id = frame.tab;
            // Ignore obsolete callbacks after the tab has been closed.
            if !update.snapshot.tabs.iter().any(|tab| tab.id == id) { continue; }
            let started = Instant::now();
            let applied = if let Some(surface) = image_cache.get_mut(&id) {
                // A resize changes both dimensions; only a new Full frame is valid.
                if surface.matches_size(&frame) {
                    surface.apply(frame)
                } else {
                    RgbSurface::from_frame(frame).map(|(new, applied)| {
                        *surface = new;
                        applied
                    })
                }
            } else {
                RgbSurface::from_frame(frame).map(|(surface, applied)| {
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
                } else if perf_enabled { background_updated += 1; }
                if perf_enabled {
                    converted_pixels += applied.converted_pixels;
                    converted_updates += 1;
                }
            } else if perf_enabled { presentation_skipped += 1; }
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
            render(&ui, &state.borrow(), &mut image_cache, &shown_tab, engine_name);
        }
        // Update the Image only when the active surface actually changed.
        if new_active_frame && active == state.borrow().active
            && (!changed || previously_shown == active) {
            if let Some(surface) = active.and_then(|id| image_cache.get(&id)) {
                ui.set_page_image(surface.image());
                ui.set_has_page(true);
                shown_tab.set(active);
            }
        }
        drop(image_cache);
        let active = state.borrow().active;
        if let Some(tab) = active {
            let (width, height) = (ui.get_viewport_width(), ui.get_viewport_height());
            let next_viewport = (tab, width, height);
            if width > 0 && height > 0 && last_viewport != Some(next_viewport) {
                dispatcher(BrowserCommand::Resize { tab, width: width as u32, height: height as u32 });
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
            stats_since = Instant::now();
            ticks = 0; presented = 0; full_presented = 0; partial_presented = 0;
            background_updated = 0; presentation_skipped = 0;
            converted_pixels = 0; converted_updates = 0;
            pump_total = Duration::ZERO; pump_max = Duration::ZERO;
            convert_total = Duration::ZERO; convert_max = Duration::ZERO;
        }
    });
    ui.run()
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
