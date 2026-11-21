//! Slint frontend. No direct dependency on Chromium or browser-core.
use browser_contracts::{
    BrowserCommand, BrowserSnapshot, BrowserTick, PageFrame, PointerAction, TabId,
};
use slint::{
    ComponentHandle, Image, ModelRc, Rgb8Pixel, SharedPixelBuffer, SharedString, VecModel,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};
slint::include_modules!();

pub type Dispatcher = Rc<dyn Fn(BrowserCommand) -> BrowserSnapshot>;
pub type Poller = Rc<dyn Fn() -> BrowserTick>;

fn convert_bgra(frame: PageFrame) -> Option<Image> {
    let bytes = usize::try_from(frame.width.checked_mul(frame.height)?.checked_mul(4)?).ok()?;
    if frame.bgra.len() != bytes || frame.width == 0 || frame.height == 0 {
        return None;
    }
    let mut pixel_buffer = SharedPixelBuffer::<Rgb8Pixel>::new(frame.width, frame.height);
    for (source, destination) in frame
        .bgra
        .chunks_exact(4)
        .zip(pixel_buffer.make_mut_bytes().chunks_exact_mut(3))
    {
        destination.copy_from_slice(&[source[2], source[1], source[0]]);
    }
    Some(Image::from_rgb8(pixel_buffer))
}

fn render(
    ui: &BrowserWindow,
    snapshot: &BrowserSnapshot,
    frames: &mut HashMap<TabId, Image>,
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
    if let Some(image) = snapshot.active.and_then(|tab| frames.get(&tab)) {
        ui.set_page_image(image.clone());
        ui.set_has_page(true);
    } else {
        ui.set_has_page(false);
    }
    frames.retain(|id, _| snapshot.tabs.iter().any(|tab| tab.id == *id));
}

fn apply_command(
    command: BrowserCommand,
    ui: &slint::Weak<BrowserWindow>,
    state: &Rc<RefCell<BrowserSnapshot>>,
    frames: &Rc<RefCell<HashMap<TabId, Image>>>,
    dispatcher: &Dispatcher,
    engine_name: &str,
) {
    let snapshot = dispatcher(command);
    let changed = *state.borrow() != snapshot;
    if changed {
        *state.borrow_mut() = snapshot;
        if let Some(ui) = ui.upgrade() {
            render(&ui, &state.borrow(), &mut frames.borrow_mut(), engine_name);
        }
    }
}

pub fn run(
    initial: BrowserSnapshot,
    dispatcher: Dispatcher,
    poller: Poller,
    engine_name: &'static str,
) -> Result<(), slint::PlatformError> {
    let ui = BrowserWindow::new()?;
    let state = Rc::new(RefCell::new(initial));
    let frames: Rc<RefCell<HashMap<TabId, Image>>> = Rc::default();
    render(&ui, &state.borrow(), &mut frames.borrow_mut(), engine_name);

    macro_rules! command_handler {
        ($name:ident, $make:expr) => {{
            let weak = ui.as_weak();
            let state = state.clone();
            let frames = frames.clone();
            let dispatcher = dispatcher.clone();
            ui.$name(move || {
                apply_command($make, &weak, &state, &frames, &dispatcher, engine_name)
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
        let dispatcher = dispatcher.clone();
        ui.on_close_tab(move |id| {
            apply_command(
                BrowserCommand::CloseTab(TabId(id as u64)),
                &weak,
                &state,
                &frames,
                &dispatcher,
                engine_name,
            )
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
        let dispatcher = dispatcher.clone();
        ui.on_select_tab(move |id| {
            apply_command(
                BrowserCommand::ActivateTab(TabId(id as u64)),
                &weak,
                &state,
                &frames,
                &dispatcher,
                engine_name,
            )
        });
    }
    {
        let weak = ui.as_weak();
        let state = state.clone();
        let frames = frames.clone();
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
        let dispatcher = dispatcher.clone();
        ui.on_back(move || {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Back(tab),
                    &weak,
                    &state,
                    &frames,
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
        let dispatcher = dispatcher.clone();
        ui.on_forward(move || {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Forward(tab),
                    &weak,
                    &state,
                    &frames,
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
        let dispatcher = dispatcher.clone();
        ui.on_reload(move || {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Reload(tab),
                    &weak,
                    &state,
                    &frames,
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
        let dispatcher = dispatcher.clone();
        ui.on_page_pointer(move |x, y, action| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                let action = match action {
                    0 => PointerAction::Down,
                    1 => PointerAction::Up,
                    _ => PointerAction::Move,
                };
                apply_command(
                    BrowserCommand::Pointer { tab, x, y, action },
                    &weak,
                    &state,
                    &frames,
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
        let dispatcher = dispatcher.clone();
        ui.on_page_scroll(move |x, y, dx, dy| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                apply_command(
                    BrowserCommand::Scroll { tab, x, y, dx, dy },
                    &weak,
                    &state,
                    &frames,
                    &dispatcher,
                    engine_name,
                );
            }
        });
    }

    // An external message pump is driven by the native Slint event loop.
    // No cross-thread CEF callbacks and no nested event loop.
    let weak = ui.as_weak();
    let mut last_viewport: Option<(TabId, i32, i32)> = None;
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            let update = poller();
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let mut image_cache = frames.borrow_mut();
            let active = update.snapshot.active;
            let mut new_active_frame = false;
            for frame in update.frames {
                let id = frame.tab;
                if let Some(image) = convert_bgra(frame) {
                    image_cache.insert(id, image);
                    new_active_frame |= Some(id) == active;
                }
            }
            let changed = *state.borrow() != update.snapshot;
            if changed {
                *state.borrow_mut() = update.snapshot;
                render(&ui, &state.borrow(), &mut image_cache, engine_name);
            } else if new_active_frame {
                if let Some(image) = active.and_then(|id| image_cache.get(&id)) {
                    ui.set_page_image(image.clone());
                    ui.set_has_page(true);
                }
            }
            drop(image_cache);
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
        },
    );
    ui.run()
}
