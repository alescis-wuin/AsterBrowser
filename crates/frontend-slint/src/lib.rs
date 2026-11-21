use std::{cell::RefCell, rc::Rc};
use browser_contracts::{BrowserCommand, BrowserSnapshot, TabId};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
slint::include_modules!();

pub type Dispatcher = Rc<dyn Fn(BrowserCommand) -> BrowserSnapshot>;

fn render(ui: &BrowserWindow, snapshot: &BrowserSnapshot) {
    let tabs: Vec<TabView> = snapshot.tabs.iter().map(|tab| TabView {
        id: tab.id.0 as i32,
        title: SharedString::from(tab.title.as_str()),
        active: snapshot.active == Some(tab.id),
    }).collect();
    ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
    let current = snapshot.tabs.iter().find(|t| Some(t.id) == snapshot.active);
    ui.set_current_url(current.map(|t| t.url.as_str()).unwrap_or("").into());
    ui.set_status_text(format!("{} onglet(s) — moteur simulé", snapshot.tabs.len()).into());
}

pub fn run(initial: BrowserSnapshot, dispatch: Dispatcher) -> Result<(), slint::PlatformError> {
    let ui = BrowserWindow::new()?;
    let state = Rc::new(RefCell::new(initial));
    render(&ui, &state.borrow());
    let install = |command: BrowserCommand, ui: &slint::Weak<BrowserWindow>, state: &Rc<RefCell<BrowserSnapshot>>, dispatch: &Dispatcher| {
        let next = dispatch(command);
        *state.borrow_mut() = next;
        if let Some(ui) = ui.upgrade() { render(&ui, &state.borrow()); }
    };
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_add_tab(move || install(BrowserCommand::NewTab { url: "about:blank".into() }, &ui_weak, &state, &dispatch));
    }
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_close_tab(move |id| install(BrowserCommand::CloseTab(TabId(id as u64)), &ui_weak, &state, &dispatch));
    }
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_select_tab(move |id| install(BrowserCommand::ActivateTab(TabId(id as u64)), &ui_weak, &state, &dispatch));
    }
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_navigate(move |text| {
            let active = state.borrow().active;
            if let Some(tab) = active {
                install(BrowserCommand::Navigate { tab, input: text.to_string() }, &ui_weak, &state, &dispatch);
            }
        });
    }
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_back(move || { let active = state.borrow().active;
            if let Some(tab) = active { install(BrowserCommand::Back(tab), &ui_weak, &state, &dispatch); } });
    }
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_forward(move || { let active = state.borrow().active;
            if let Some(tab) = active { install(BrowserCommand::Forward(tab), &ui_weak, &state, &dispatch); } });
    }
    {
        let ui_weak = ui.as_weak(); let state = state.clone(); let dispatch = dispatch.clone();
        ui.on_reload(move || { let active = state.borrow().active;
            if let Some(tab) = active { install(BrowserCommand::Reload(tab), &ui_weak, &state, &dispatch); } });
    }
    ui.run()
}
