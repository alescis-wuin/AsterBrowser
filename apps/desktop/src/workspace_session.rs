//! Desktop coordination of persistent resources and ephemeral Browser tabs.
//! Neither browser-core nor frontend-slint needs CEF or filesystem knowledge.
use browser_contracts::{
    BrowserCommand, BrowserEngine, BrowserError, BrowserSnapshot, MoveTargetView, SavedItemView,
    SearchResultView, TabId, WorkspaceCommand, WorkspaceSnapshot, WorkspaceView,
};
use browser_core::{
    Browser,
    workspaces::{Appearance, SavedResource, WorkspaceDisk, WorkspaceEntry, WorkspaceError, WorkspaceStore,
                 rank_metadata, search_terms},
};
use std::collections::HashMap;

#[derive(Clone, Copy)]
struct TabBinding {
    workspace_id: u64,
    resource_id: Option<u64>,
}

pub struct WorkspaceSession {
    pub store: WorkspaceStore,
    disk: WorkspaceDisk,
    readonly: bool,
    bindings: HashMap<TabId, TabBinding>,
    recently_active: HashMap<u64, TabId>,
    message: String,
    revision: u64,
    search_query: String,
    search_scope: i32,
}

fn palette_index(color: &str) -> i32 {
    ["#9ddac8", "#bdacfc", "#a7c9ee", "#e5c9a2", "#e7aaa9"]
        .iter().position(|item| *item == color).unwrap_or(0) as i32
}
fn icon_index(icon: &str) -> i32 {
    ["grid", "code", "film", "book", "star", "folder", "globe", "file", "cloud", "terminal", "pin", "link"]
        .iter().position(|item| *item == icon).unwrap_or(0) as i32
}
pub fn appearance_from_indices(palette: i32, icon: i32) -> (String, String) {
    let colors = ["#9ddac8", "#bdacfc", "#a7c9ee", "#e5c9a2", "#e7aaa9"];
    let icons = ["grid", "code", "film", "book", "star", "folder", "globe", "file", "cloud", "terminal", "pin", "link"];
    (colors.get(palette as usize).unwrap_or(&colors[0]).to_string(),
     icons.get(icon as usize).unwrap_or(&icons[0]).to_string())
}

impl WorkspaceSession {
    pub fn load() -> Self {
        let disk = WorkspaceDisk::new(WorkspaceDisk::default_path());
        let (store, readonly, message) = match disk.load_or_default() {
            Ok(store) => (store, false, String::new()),
            Err(err) => (
                WorkspaceStore::default(), true,
                format!("Sauvegarde invalide : {err}. Mode lecture seule ; aucune donnée remplacée."),
            ),
        };
        Self { store, disk, readonly, bindings: HashMap::new(), recently_active: HashMap::new(), message, revision: 1, search_query: String::new(), search_scope: 0 }
    }
    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self { store: WorkspaceStore::default(), disk: WorkspaceDisk::new(std::env::temp_dir().join("aster-should-not-write.json")), readonly: true, bindings: HashMap::new(), recently_active: HashMap::new(), message: String::new(), revision: 1, search_query: String::new(), search_scope: 0 }
    }
    pub fn revision(&self) -> u64 { self.revision }
    pub fn reconcile(&mut self, snapshot: &BrowserSnapshot) {
        let before = self.bindings.len();
        self.bindings.retain(|tab, _| snapshot.tabs.iter().any(|t| t.id == *tab));
        for tab in &snapshot.tabs {
            self.bindings.entry(tab.id).or_insert(TabBinding {
                workspace_id: self.store.active_workspace_id, resource_id: None,
            });
        }
        if self.bindings.len() != before { self.revision = self.revision.wrapping_add(1); }
    }
    pub fn attach_initial(&mut self, browser: &BrowserSnapshot) {
        let workspace_id = self.store.active_workspace_id;
        for tab in &browser.tabs {
            self.bindings.entry(tab.id).or_insert(TabBinding { workspace_id, resource_id: None });
        }
        if let Some(active) = browser.active { self.recently_active.insert(workspace_id, active); }
        self.revision = self.revision.wrapping_add(1);
    }
    fn write(&mut self) {
        if self.readonly { return; }
        if let Err(e) = self.disk.write(&self.store) {
            self.message = format!("Échec d'enregistrement : {e}");
            eprintln!("[aster:spaces] {}", self.message);
        }
    }
    fn select_visible<E: BrowserEngine>(&mut self, browser: &mut Browser<E>) -> Result<(), BrowserError> {
        let ws = self.store.active_workspace_id;
        let snapshot = browser.snapshot();
        let candidate = self.recently_active.get(&ws).copied()
            .filter(|id| snapshot.tabs.iter().any(|t| t.id == *id) && self.bindings.get(id).is_some_and(|b| b.workspace_id == ws))
            .or_else(|| snapshot.tabs.iter().find(|t| self.bindings.get(&t.id).is_some_and(|b| b.workspace_id == ws)).map(|t| t.id));
        if let Some(id) = candidate {
            if snapshot.active != Some(id) { browser.dispatch(BrowserCommand::ActivateTab(id))?; }
            self.recently_active.insert(ws, id);
        } else { browser.clear_active(); }
        Ok(())
    }
    /// CEF commands retain their existing interface. Only successful tab transitions
    /// change bindings; no persisted TabId leaks into workspace JSON.
    pub fn dispatch_browser<E: BrowserEngine>(&mut self, browser: &mut Browser<E>, cmd: BrowserCommand) -> Result<(), BrowserError> {
        let old = browser.snapshot();
        browser.dispatch(cmd.clone())?;
        let next = browser.snapshot();
        let reorganized = matches!(&cmd, BrowserCommand::NewTab { .. } | BrowserCommand::CloseTab(_) | BrowserCommand::ActivateTab(_));
        match cmd {
            BrowserCommand::NewTab { .. } => {
                if let Some(tab) = next.tabs.iter().find(|tab| !old.tabs.iter().any(|prev| prev.id == tab.id)) {
                    let ws = self.store.active_workspace_id;
                    self.bindings.insert(tab.id, TabBinding { workspace_id: ws, resource_id: None });
                    self.recently_active.insert(ws, tab.id);
                }
            }
            BrowserCommand::CloseTab(id) => {
                self.bindings.remove(&id);
                self.recently_active.retain(|_, tab_id| *tab_id != id);
                // Browser's generic fallback can be an unrelated space. Correct it.
                self.select_visible(browser)?;
            }
            BrowserCommand::ActivateTab(id) => {
                if let Some(binding) = self.bindings.get(&id).copied() {
                    if self.store.active_workspace_id != binding.workspace_id {
                        if self.store.activate(binding.workspace_id).is_ok() { self.write(); }
                    }
                    self.recently_active.insert(binding.workspace_id, id);
                }
            }
            _ => {}
        }
        if reorganized { self.revision = self.revision.wrapping_add(1); }
        Ok(())
    }
    fn ensure_saved_tab<E: BrowserEngine>(&mut self, browser: &Browser<E>, id: TabId) -> Result<u64, WorkspaceError> {
        let tab = browser.snapshot().tabs.into_iter().find(|tab| tab.id == id)
            .ok_or(WorkspaceError::NotFound("Onglet introuvable"))?;
        let workspace_id = self.bindings.get(&id).map_or(self.store.active_workspace_id, |binding| binding.workspace_id);
        let title = if matches!(tab.title.as_str(), "" | "Loading..." | "New tab") {
            tab.url.clone()
        } else {
            tab.title
        };
        let title: String = title.chars().take(80).collect();
        let entry_id = self.store.save_resource(workspace_id, &tab.url, &title, None)?;
        let resource_id = self.store.entry(entry_id).expect("saved entry exists").resource_id;
        if let Some(binding) = self.bindings.get_mut(&id) { binding.resource_id = Some(resource_id); }
        Ok(entry_id)
    }

    /// Moving a *live tab* never closes or reloads CEF. Its destination gains a
    /// saved reference while source bookmarks remain untouched.
    fn move_live_tab<E: BrowserEngine>(&mut self, browser: &Browser<E>, tab_id: TabId,
        workspace_id: u64, folder_id: Option<u64>) -> Result<(), WorkspaceError> {
        let tab = browser.snapshot().tabs.into_iter().find(|tab| tab.id == tab_id)
            .ok_or(WorkspaceError::NotFound("Onglet introuvable"))?;
        let source = *self.bindings.get(&tab_id).ok_or(WorkspaceError::NotFound("Association d'onglet introuvable"))?;
        let title = if matches!(tab.title.as_str(), "" | "Loading..." | "New tab") {
            tab.url.clone()
        } else { tab.title };
        let mut draft = self.store.clone();
        let entry_id = draft.save_resource(workspace_id, &tab.url,
            &title.chars().take(80).collect::<String>(), folder_id)?;
        // A destination entry may already exist; the explicit folder choice wins.
        draft.move_entry(entry_id, workspace_id, folder_id)?;
        let resource_id = draft.entry(entry_id).expect("saved entry exists").resource_id;
        if browser.snapshot().active == Some(tab_id) { draft.activate(workspace_id)?; }
        self.store = draft;
        self.bindings.insert(tab_id, TabBinding { workspace_id, resource_id: Some(resource_id) });
        if self.recently_active.get(&source.workspace_id) == Some(&tab_id) {
            self.recently_active.remove(&source.workspace_id);
        }
        self.recently_active.insert(workspace_id, tab_id);
        Ok(())
    }

    fn search_results(&self, browser: &BrowserSnapshot) -> Vec<SearchResultView> {
        let terms = search_terms(&self.search_query);
        if terms.is_empty() { return Vec::new(); }
        let mut hits: Vec<(i32, SearchResultView)> = Vec::new();
        if self.search_scope == 0 {
            for ws in &self.store.workspaces {
                if let Some(score) = rank_metadata(&terms, &ws.name, "", &[], "") {
                    hits.push((score, SearchResultView {
                        kind: 2, workspace_id: ws.id, title: ws.name.clone(),
                        subtitle: "Espace de travail".into(), icon: ws.appearance.icon.clone(),
                        color: ws.appearance.color.clone(), ..Default::default()
                    }));
                }
            }
        }
        if matches!(self.search_scope, 0 | 1) {
            for tab in &browser.tabs {
                let Some(binding) = self.bindings.get(&tab.id) else { continue; };
                let (tags, notes) = binding.resource_id.and_then(|id| self.store.resource(id))
                    .map_or((Vec::new(), String::new()), |r| (r.tags.clone(), r.notes.clone()));
                if let Some(score) = rank_metadata(&terms, &tab.title, &tab.url, &tags, &notes) {
                    let ws_name = self.store.workspace(binding.workspace_id).map_or("", |w| w.name.as_str());
                    hits.push((score + 1, SearchResultView {
                        kind: 0, workspace_id: binding.workspace_id, tab_id: Some(tab.id),
                        title: tab.title.clone(), subtitle: format!("Onglet · {ws_name} · {}", tab.url),
                        icon: "globe".into(), color: "#a7c9ee".into(), ..Default::default()
                    }));
                }
            }
        }
        if matches!(self.search_scope, 0 | 2 | 3) {
            // Distinct per workspace; one resource may legitimately belong to many spaces.
            let mut seen = std::collections::HashSet::new();
            for entry in &self.store.entries {
                let Some(resource) = self.store.resource(entry.resource_id) else { continue; };
                if self.search_scope == 3 && !resource.favorite { continue; }
                if !seen.insert((entry.workspace_id, resource.id)) { continue; }
                if let Some(score) = rank_metadata(&terms, &resource.title, &resource.url, &resource.tags, &resource.notes) {
                    let ws_name = self.store.workspace(entry.workspace_id).map_or("", |w| w.name.as_str());
                    hits.push((score + if resource.favorite { 2 } else { 0 }, SearchResultView {
                        kind: 1, workspace_id: entry.workspace_id, resource_id: Some(resource.id),
                        title: resource.title.clone(), subtitle: format!("{ws_name} · {}", resource.url),
                        icon: resource.appearance.icon.clone(), color: resource.appearance.color.clone(),
                        ..Default::default()
                    }));
                }
            }
            for resource in self.store.resources.iter().filter(|r| r.favorite
                && !self.store.entries.iter().any(|e| e.resource_id == r.id)) {
                if let Some(score) = rank_metadata(&terms, &resource.title, &resource.url, &resource.tags, &resource.notes) {
                    hits.push((score + 2, SearchResultView {
                        kind: 1, workspace_id: self.store.active_workspace_id,
                        resource_id: Some(resource.id), title: resource.title.clone(),
                        subtitle: resource.url.clone(), icon: resource.appearance.icon.clone(),
                        color: resource.appearance.color.clone(), ..Default::default()
                    }));
                }
            }
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0)
            .then_with(|| a.1.title.cmp(&b.1.title))
            .then_with(|| a.1.workspace_id.cmp(&b.1.workspace_id)));
        hits.into_iter().take(40).map(|(_, result)| result).collect()
    }

    fn resource_row(&self, e: Option<&WorkspaceEntry>, r: &SavedResource, depth: usize) -> SavedItemView {
        let workspace = e.map_or(self.store.active_workspace_id, |entry| entry.workspace_id);
        SavedItemView {
            id: e.map_or(0, |e| e.id), resource_id: r.id, folder: false,
            title: r.title.clone(), subtitle: r.url.clone(), notes: r.notes.clone(),
            tags: r.tags.join(", "), icon: r.appearance.icon.clone(), color: r.appearance.color.clone(),
            palette: palette_index(&r.appearance.color), icon_index: icon_index(&r.appearance.icon),
            depth, pinned: e.is_some_and(|e| e.pinned), favorite: r.favorite,
            live: self.bindings.values().any(|b| b.workspace_id == workspace && b.resource_id == Some(r.id)),
        }
    }
    fn collect_entries(&self, workspace_id: u64, folder_id: Option<u64>, depth: usize, out: &mut Vec<SavedItemView>) {
        let mut entries: Vec<_> = self.store.entries.iter().filter(|e| e.workspace_id == workspace_id && e.folder_id == folder_id).collect();
        entries.sort_by_key(|e| (!e.pinned, e.order, e.id));
        for entry in entries {
            if let Some(r) = self.store.resource(entry.resource_id) {
                out.push(self.resource_row(Some(entry), r, depth));
            }
        }
        let mut folders: Vec<_> = self.store.collections.iter().filter(|f| f.workspace_id == workspace_id && f.parent_id == folder_id).collect();
        folders.sort_by_key(|f| (f.order, f.id));
        for f in folders {
            out.push(SavedItemView {
                id: f.id, folder: true, title: f.name.clone(),
                subtitle: "Dossier virtuel".into(), color: f.appearance.color.clone(),
                icon: f.appearance.icon.clone(), depth,
                palette: palette_index(&f.appearance.color), icon_index: icon_index(&f.appearance.icon),
                ..SavedItemView::default()
            });
            self.collect_entries(workspace_id, Some(f.id), depth + 1, out);
        }
    }
    pub fn projection(&self, browser: &BrowserSnapshot) -> WorkspaceSnapshot {
        let active = self.store.active_workspace_id;
        let mut workspaces: Vec<_> = self.store.workspaces.iter().map(|w| WorkspaceView {
            id: w.id, name: w.name.clone(), icon: w.appearance.icon.clone(),
            color: w.appearance.color.clone(),
            palette: palette_index(&w.appearance.color), icon_index: icon_index(&w.appearance.icon),
            count: self.store.entries.iter().filter(|e| e.workspace_id == w.id).count(),
            active: w.id == active,
        }).collect();
        workspaces.sort_by_key(|w| self.store.workspace(w.id).unwrap().order);
        let visible_tab_ids = self.bindings.iter().filter_map(|(tab, binding)| (binding.workspace_id == active).then_some(*tab)).collect();
        let mut items = Vec::new();
        self.collect_entries(active, None, 0, &mut items);
        let favorites = self.store.favorites().into_iter().map(|r| {
            // Prefer a reference belonging to the current space, if one exists.
            let e = self.store.entries.iter().find(|e| e.resource_id == r.id && e.workspace_id == active)
                .or_else(|| self.store.entries.iter().find(|e| e.resource_id == r.id));
            self.resource_row(e, r, 0)
        }).collect();
        let mut destinations = Vec::new();
        for ws in &workspaces {
            destinations.push(MoveTargetView { workspace_id: ws.id, folder_id: None, label: ws.name.clone() + " / racine", depth: 0 });
            for (folder, depth) in self.store.folders_depth_first(ws.id) {
                destinations.push(MoveTargetView { workspace_id: ws.id, folder_id: Some(folder.id), label: ws.name.clone() + " / " + &folder.name, depth: depth + 1 });
            }
        }
        WorkspaceSnapshot { active_workspace_id: active, workspaces, visible_tab_ids, items, favorites,
            destinations, search_results: self.search_results(browser), message: self.message.clone() }
    }
    fn open_resource<E: BrowserEngine>(&mut self, browser: &mut Browser<E>, id: u64) -> Result<(), String> {
        let resource = self.store.resource(id).ok_or("Ressource introuvable")?.clone();
        let workspace_id = self.store.active_workspace_id;
        if let Some(tab) = browser.snapshot().tabs.iter().find(|tab| self.bindings.get(&tab.id).is_some_and(|b| b.workspace_id == workspace_id && b.resource_id == Some(id))) {
            self.dispatch_browser(browser, BrowserCommand::ActivateTab(tab.id)).map_err(|e| e.to_string())?;
            return Ok(());
        }
        self.dispatch_browser(browser, BrowserCommand::NewTab { url: resource.url }).map_err(|e| e.to_string())?;
        if let Some(active) = browser.snapshot().active {
            self.bindings.insert(active, TabBinding { workspace_id, resource_id: Some(id) });
        }
        Ok(())
    }
    fn run_command<E: BrowserEngine>(&mut self, browser: &mut Browser<E>, cmd: WorkspaceCommand) -> Result<(), String> {
        use WorkspaceCommand::*;
        if let WorkspaceCommand::OpenResource(id) = &cmd {
            return self.open_resource(browser, *id);
        }
        let result: Result<(), WorkspaceError> = (|| {
        match cmd {
            CreateWorkspace { title } => { self.store.create_workspace(&title).map(|_| ()) }
            RenameWorkspace { id, title, color, icon } => {
                let mut draft = self.store.clone();
                draft.rename_workspace(id, &title)?;
                draft.set_appearance(id, Appearance { color, icon })?;
                self.store = draft;
                Ok(())
            }
            DeleteWorkspace(id) => {
                let destination = self.store.remove_workspace(id)?;
                for binding in self.bindings.values_mut() {
                    if binding.workspace_id == id { binding.workspace_id = destination; }
                }
                self.recently_active.remove(&id);
                Ok(())
            }
            ReorderWorkspace { id, direction } => self.store.reorder_workspace(id, direction),
            ActivateWorkspace(id) => self.store.activate(id),
            CreateFolder { parent_id, title } => self.store.create_folder(self.store.active_workspace_id, parent_id, &title).map(|_| ()),
            EditFolder { id, title, color, icon } => {
                let mut draft = self.store.clone();
                draft.rename_folder(id, &title)?;
                draft.set_appearance(id, Appearance { color, icon })?;
                self.store = draft;
                Ok(())
            }
            DeleteFolder(id) => self.store.remove_folder(id),
            MoveFolder { id, parent_id } => self.store.move_folder(id, self.store.active_workspace_id, parent_id),
            SaveTab(id) => self.ensure_saved_tab(browser, id).map(|_| ()),
            MoveTab { tab, workspace_id, folder_id } => {
                self.move_live_tab(browser, tab, workspace_id, folder_id)
            }
            ToggleTabPin(id) => {
                let entry_id = self.ensure_saved_tab(browser, id)?;
                let pinned = self.store.entry(entry_id).expect("saved entry exists").pinned;
                self.store.set_pin(entry_id, !pinned)
            }
            ToggleTabFavorite(id) => {
                let entry_id = self.ensure_saved_tab(browser, id)?;
                let resource_id = self.store.entry(entry_id).expect("saved entry exists").resource_id;
                let favorite = self.store.resource(resource_id).expect("saved resource exists").favorite;
                self.store.set_favorite(resource_id, !favorite)
            }
            Search { .. } => unreachable!("Search is handled without writes"),
            AddLink { url, title } => {
                let title = if title.trim().is_empty() { url.chars().take(80).collect() } else { title };
                self.store.save_resource(self.store.active_workspace_id, &url, &title, None).map(|_| ())
            }
            OpenResource(_) => unreachable!(),
            RemoveEntry(id) => {
                let resource = self.store.entry(id).map(|e| e.resource_id);
                self.store.remove_entry(id)?;
                if let Some(resource) = resource {
                    if self.store.entries.iter().all(|entry| entry.resource_id != resource) {
                        for binding in self.bindings.values_mut() {
                            if binding.resource_id == Some(resource) { binding.resource_id = None; }
                        }
                    }
                }
                Ok(())
            }
            MoveEntry { id, workspace_id, folder_id } => {
                let old = self.store.entry(id).cloned().ok_or(WorkspaceError::NotFound("Entrée introuvable"))?;
                self.store.move_entry(id, workspace_id, folder_id)?;
                for binding in self.bindings.values_mut() {
                    if binding.workspace_id == old.workspace_id && binding.resource_id == Some(old.resource_id) {
                        binding.workspace_id = workspace_id;
                    }
                }
                Ok(())
            }
            ReorderEntry { id, direction } => self.store.reorder_entry(id, direction),
            TogglePin(id) => {
                let entry = self.store.entry(id).ok_or(WorkspaceError::NotFound("Entrée introuvable"))?;
                self.store.set_pin(id, !entry.pinned)
            }
            ToggleFavorite(id) => {
                let resource = self.store.resource(id).ok_or(WorkspaceError::NotFound("Ressource introuvable"))?;
                self.store.set_favorite(id, !resource.favorite)
            }
            EditResource { id, title, notes, tags, color, icon } => {
                self.store.edit_resource(id, &title, &notes, tags.split(',').map(str::to_owned).collect(), Appearance { color, icon })
            }
        }
        })();
        result.map_err(|e| e.to_string())?;
        self.select_visible(browser).map_err(|e| e.to_string())?;
        self.message.clear();
        self.write();
        Ok(())
    }
    pub fn dispatch_workspace<E: BrowserEngine>(&mut self, browser: &mut Browser<E>, cmd: WorkspaceCommand) {
        if let WorkspaceCommand::Search { query, scope } = cmd {
            self.search_query = query.chars().take(160).collect();
            self.search_scope = scope.clamp(0, 3);
            self.revision = self.revision.wrapping_add(1);
            return;
        }
        if self.readonly {
            self.message = "Le stockage est en lecture seule : corriger la sauvegarde avant toute modification".into();
            return;
        }
        let old_store = self.store.clone();
        let old_bindings = self.bindings.clone();
        let old_recent = self.recently_active.clone();
        if let Err(e) = self.run_command(browser, cmd) {
            self.store = old_store;
            self.bindings = old_bindings;
            self.recently_active = old_recent;
            self.message = format!("Action refusée : {e}");
        }
        self.revision = self.revision.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use browser_core::MockEngine;
    #[test]
    fn saved_tab_survives_close_and_can_be_opened_lazily() {
        let mut browser = Browser::new(MockEngine::default());
        let mut session = WorkspaceSession::in_memory();
        session.dispatch_browser(&mut browser, BrowserCommand::NewTab { url: "https://example.org".into() }).unwrap();
        let tab = browser.snapshot().active.unwrap();
        let store_id = session.store.save_resource(1, "https://example.org/", "Exemple", None).unwrap();
        let rid = session.store.entry(store_id).unwrap().resource_id;
        session.bindings.get_mut(&tab).unwrap().resource_id = Some(rid);
        session.dispatch_browser(&mut browser, BrowserCommand::CloseTab(tab)).unwrap();
        assert!(browser.snapshot().tabs.is_empty());
        assert_eq!(session.store.resources.len(), 1);
        session.open_resource(&mut browser, rid).unwrap();
        assert_eq!(browser.snapshot().tabs.len(), 1);
    }
    #[test]
    fn moving_live_tab_preserves_tab_id_and_creates_reopenable_destination() {
        let mut browser = Browser::new(MockEngine::default());
        let mut session = WorkspaceSession::in_memory();
        session.dispatch_browser(&mut browser, BrowserCommand::NewTab { url: "https://example.org".into() }).unwrap();
        let tab = browser.snapshot().active.unwrap();
        let original_entry = session.store.save_resource(1, "https://example.org/", "Source", None).unwrap();
        let target = session.store.create_workspace("Équipe").unwrap();
        let folder = session.store.create_folder(target, None, "À lire").unwrap();
        session.store.activate(1).unwrap();
        session.run_command(&mut browser, WorkspaceCommand::MoveTab { tab, workspace_id: target, folder_id: Some(folder) }).unwrap();
        assert_eq!(browser.snapshot().active, Some(tab));
        assert_eq!(browser.snapshot().tabs.len(), 1);
        assert_eq!(session.store.active_workspace_id, target);
        assert!(session.store.entry(original_entry).is_some());
        let entry = session.store.entries.iter().find(|e| e.workspace_id == target).unwrap();
        assert_eq!(entry.folder_id, Some(folder));
        let entry_id = entry.id;
        assert_eq!(session.bindings.get(&tab).unwrap().workspace_id, target);
        session.dispatch_browser(&mut browser, BrowserCommand::CloseTab(tab)).unwrap();
        assert!(session.store.entry(entry_id).is_some());
    }
    #[test]
    fn tab_actions_auto_save_and_search_reaches_other_workspaces() {
        let mut browser = Browser::new(MockEngine::default());
        let mut session = WorkspaceSession::in_memory();
        session.dispatch_browser(&mut browser, BrowserCommand::NewTab { url: "https://example.org".into() }).unwrap();
        let tab = browser.snapshot().active.unwrap();
        session.run_command(&mut browser, WorkspaceCommand::ToggleTabPin(tab)).unwrap();
        session.run_command(&mut browser, WorkspaceCommand::ToggleTabFavorite(tab)).unwrap();
        assert!(session.store.entries[0].pinned);
        assert!(session.store.resources[0].favorite);
        let next = session.store.create_workspace("Recherche").unwrap();
        session.store.activate(next).unwrap();
        session.search_query = "example".into();
        session.search_scope = 3;
        let hits = session.projection(&browser.snapshot()).search_results;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].resource_id, Some(session.store.resources[0].id));
        session.search_scope = 0;
        assert_eq!(session.projection(&browser.snapshot()).search_results.len(), 2);
    }
    #[test]
    fn invalid_tab_move_keeps_existing_binding_and_store() {
        let mut browser = Browser::new(MockEngine::default());
        let mut session = WorkspaceSession::in_memory();
        session.dispatch_browser(&mut browser, BrowserCommand::NewTab { url: "https://example.org".into() }).unwrap();
        let tab = browser.snapshot().active.unwrap();
        let original = session.store.clone();
        assert!(session.move_live_tab(&browser, tab, 999, Some(5)).is_err());
        assert_eq!(session.store, original);
        assert_eq!(session.bindings.get(&tab).unwrap().workspace_id, 1);
    }
    #[test]
    fn workspace_switch_does_not_close_other_tabs() {
        let mut browser = Browser::new(MockEngine::default());
        let mut session = WorkspaceSession::in_memory();
        session.dispatch_browser(&mut browser, BrowserCommand::NewTab { url: "https://example.org".into() }).unwrap();
        let first = browser.snapshot().active.unwrap();
        let next = session.store.create_workspace("Travail").unwrap();
        session.select_visible(&mut browser).unwrap();
        assert_eq!(browser.snapshot().active, None);
        session.dispatch_browser(&mut browser, BrowserCommand::NewTab { url: "https://example.net".into() }).unwrap();
        session.store.activate(1).unwrap();
        session.select_visible(&mut browser).unwrap();
        assert_eq!(browser.snapshot().active, Some(first));
        assert_eq!(browser.snapshot().tabs.len(), 2);
        assert_eq!(session.projection(&browser.snapshot()).visible_tab_ids.len(), 1);
        assert_eq!(next, 2);
    }
}
