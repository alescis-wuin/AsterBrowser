//! Persistent, engine-independent workspace organization.
//! Live Chromium tabs and cookies are never serialized here.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const SCHEMA: u32 = 1;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SPACES: usize = 50;
const MAX_FOLDERS: usize = 500;
const MAX_RESOURCES: usize = 1000;
const MAX_ENTRIES: usize = 3000;
const MAX_DEPTH: usize = 12;
const ICONS: &[&str] = &["grid", "code", "film", "book", "star", "folder", "globe", "file", "cloud", "terminal", "pin", "link"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub color: String,
    pub icon: String,
}
impl Default for Appearance {
    fn default() -> Self {
        Self { color: "#9ddac8".into(), icon: "grid".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: u64,
    pub name: String,
    pub order: u64,
    pub appearance: Appearance,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub id: u64,
    pub workspace_id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub order: u64,
    pub appearance: Appearance,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedResource {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub appearance: Appearance,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEntry {
    pub id: u64,
    pub workspace_id: u64,
    pub resource_id: u64,
    pub folder_id: Option<u64>,
    pub pinned: bool,
    pub order: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceStore {
    pub version: u32,
    pub next_id: u64,
    pub active_workspace_id: u64,
    pub workspaces: Vec<Workspace>,
    pub collections: Vec<Collection>,
    pub resources: Vec<SavedResource>,
    pub entries: Vec<WorkspaceEntry>,
}

#[derive(Debug)]
pub enum WorkspaceError {
    Invalid(&'static str),
    NotFound(&'static str),
    Io(std::io::Error),
    Json(serde_json::Error),
}
impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::NotFound(message) => write!(f, "{message}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for WorkspaceError {}
impl From<std::io::Error> for WorkspaceError {
    fn from(value: std::io::Error) -> Self { Self::Io(value) }
}
impl From<serde_json::Error> for WorkspaceError {
    fn from(value: serde_json::Error) -> Self { Self::Json(value) }
}

type Result<T> = std::result::Result<T, WorkspaceError>;

fn checked_title(input: &str) -> Result<String> {
    let name = input.trim();
    if name.is_empty() || name.len() > 100 || name.chars().any(char::is_control) {
        return Err(WorkspaceError::Invalid("Le nom doit contenir entre 1 et 100 octets imprimables"));
    }
    Ok(name.to_owned())
}
fn checked_appearance(input: &Appearance) -> Result<()> {
    if input.color.len() != 7 || !input.color.starts_with('#')
        || !input.color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        || !ICONS.contains(&input.icon.as_str())
    {
        return Err(WorkspaceError::Invalid("Couleur ou icône non autorisée"));
    }
    Ok(())
}
fn checked_url(input: &str) -> Result<String> {
    let input = input.trim();
    if input == "about:blank" { return Ok(input.to_owned()); }
    if input.len() > 4096 || input.chars().any(char::is_control) {
        return Err(WorkspaceError::Invalid("URL trop longue ou malformée"));
    }
    let parsed = url::Url::parse(input).map_err(|_| WorkspaceError::Invalid("URL absolue HTTP(S) requise"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none()
        || !parsed.username().is_empty() || parsed.password().is_some()
    {
        return Err(WorkspaceError::Invalid("Seules les URL HTTP(S) sans identifiants sont autorisées"));
    }
    Ok(parsed.to_string())
}
fn checked_tags(input: Vec<String>) -> Result<Vec<String>> {
    if input.len() > 12 { return Err(WorkspaceError::Invalid("Trop de mots-clés")); }
    let mut tags = Vec::new();
    for raw in input {
        let tag = raw.trim().to_lowercase();
        if tag.is_empty() { continue; }
        if tag.len() > 40 || tag.chars().any(char::is_control) {
            return Err(WorkspaceError::Invalid("Mot-clé invalide"));
        }
        if !tags.contains(&tag) { tags.push(tag); }
    }
    Ok(tags)
}

impl Default for WorkspaceStore {
    fn default() -> Self {
        Self {
            version: SCHEMA,
            next_id: 2,
            active_workspace_id: 1,
            workspaces: vec![Workspace {
                id: 1, name: "Navigation".into(), order: 0,
                appearance: Appearance::default(),
            }],
            collections: Vec::new(), resources: Vec::new(), entries: Vec::new(),
        }
    }
}

impl WorkspaceStore {
    fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn active_workspace(&self) -> u64 { self.active_workspace_id }
    pub fn resource(&self, id: u64) -> Option<&SavedResource> {
        self.resources.iter().find(|r| r.id == id)
    }
    pub fn entry(&self, id: u64) -> Option<&WorkspaceEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn folder(&self, id: u64) -> Option<&Collection> {
        self.collections.iter().find(|f| f.id == id)
    }
    pub fn workspace(&self, id: u64) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }
    fn valid_parent(&self, workspace_id: u64, parent_id: Option<u64>) -> Result<()> {
        if self.workspace(workspace_id).is_none() { return Err(WorkspaceError::NotFound("Espace introuvable")); }
        if let Some(parent) = parent_id {
            if !self.folder(parent).is_some_and(|f| f.workspace_id == workspace_id) {
                return Err(WorkspaceError::Invalid("Dossier parent absent ou étranger à l'espace"));
            }
        }
        Ok(())
    }
    /// Validate all references and fields before accepting a disk snapshot.
    pub fn validate(&self) -> Result<()> {
        if self.version != SCHEMA { return Err(WorkspaceError::Invalid("Version de sauvegarde inconnue")); }
        if self.workspaces.is_empty() || self.workspaces.len() > MAX_SPACES
            || self.collections.len() > MAX_FOLDERS || self.resources.len() > MAX_RESOURCES
            || self.entries.len() > MAX_ENTRIES
        { return Err(WorkspaceError::Invalid("Limites de stockage dépassées")); }
        let mut ids = HashSet::new();
        for w in &self.workspaces {
            if !ids.insert(w.id) || w.id == 0 { return Err(WorkspaceError::Invalid("ID dupliqué ou nul")); }
            checked_title(&w.name)?;
            checked_appearance(&w.appearance)?;
        }
        if self.workspace(self.active_workspace_id).is_none() {
            return Err(WorkspaceError::Invalid("Espace actif introuvable"));
        }
        for f in &self.collections {
            if !ids.insert(f.id) || f.id == 0 { return Err(WorkspaceError::Invalid("ID dupliqué ou nul")); }
            checked_title(&f.name)?;
            checked_appearance(&f.appearance)?;
            self.valid_parent(f.workspace_id, f.parent_id)?;
            let mut seen = HashSet::from([f.id]);
            let mut cur = f.parent_id;
            while let Some(parent_id) = cur {
                if !seen.insert(parent_id) || seen.len() > MAX_DEPTH {
                    return Err(WorkspaceError::Invalid("Cycle ou profondeur de dossier excessive"));
                }
                cur = self.folder(parent_id).ok_or(WorkspaceError::Invalid("Dossier parent introuvable"))?.parent_id;
            }
        }
        for r in &self.resources {
            if !ids.insert(r.id) || r.id == 0 { return Err(WorkspaceError::Invalid("ID dupliqué ou nul")); }
            checked_title(&r.title)?;
            checked_url(&r.url)?;
            if r.notes.len() > 2000 || r.notes.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                return Err(WorkspaceError::Invalid("Note trop longue ou invalide"));
            }
            checked_tags(r.tags.clone())?;
            checked_appearance(&r.appearance)?;
        }
        for e in &self.entries {
            if !ids.insert(e.id) || e.id == 0 { return Err(WorkspaceError::Invalid("ID dupliqué ou nul")); }
            if self.resource(e.resource_id).is_none() { return Err(WorkspaceError::Invalid("Ressource référencée absente")); }
            self.valid_parent(e.workspace_id, e.folder_id)?;
        }
        if self.next_id > i32::MAX as u64 || self.next_id <= *ids.iter().max().unwrap_or(&0) {
            return Err(WorkspaceError::Invalid("Compteur d'identifiants invalide"));
        }
        Ok(())
    }
    /// Copy/validate/commit: invalid operations leave the original state unchanged.
    pub fn edit<T>(&mut self, action: impl FnOnce(&mut WorkspaceStore) -> Result<T>) -> Result<T> {
        let mut draft = self.clone();
        let result = action(&mut draft)?;
        draft.validate()?;
        *self = draft;
        Ok(result)
    }
    pub fn create_workspace(&mut self, title: &str) -> Result<u64> {
        let title = checked_title(title)?;
        self.edit(|s| {
            if s.workspaces.len() >= MAX_SPACES { return Err(WorkspaceError::Invalid("Limite d'espaces atteinte")); }
            let id = s.allocate();
            let order = s.workspaces.iter().map(|w| w.order).max().unwrap_or(0) + 1;
            s.workspaces.push(Workspace { id, name: title, order, appearance: Appearance::default() });
            s.active_workspace_id = id;
            Ok(id)
        })
    }
    pub fn rename_workspace(&mut self, id: u64, title: &str) -> Result<()> {
        let title = checked_title(title)?;
        self.edit(|s| {
            s.workspaces.iter_mut().find(|w| w.id == id)
                .ok_or(WorkspaceError::NotFound("Espace introuvable"))?.name = title;
            Ok(())
        })
    }
    /// Entries are relocated to the default workspace rather than deleted.
    pub fn remove_workspace(&mut self, id: u64) -> Result<u64> {
        self.edit(|s| {
            if s.workspaces.len() <= 1 { return Err(WorkspaceError::Invalid("Dernier espace non supprimable")); }
            let pos = s.workspaces.iter().position(|w| w.id == id)
                .ok_or(WorkspaceError::NotFound("Espace introuvable"))?;
            s.workspaces.remove(pos);
            let destination = s.workspaces[0].id;
            for e in &mut s.entries {
                if e.workspace_id == id { e.workspace_id = destination; e.folder_id = None; }
            }
            s.collections.retain(|f| f.workspace_id != id);
            if s.active_workspace_id == id { s.active_workspace_id = destination; }
            Ok(destination)
        })
    }
    pub fn activate(&mut self, id: u64) -> Result<()> {
        self.edit(|s| {
            if s.workspace(id).is_none() { return Err(WorkspaceError::NotFound("Espace introuvable")); }
            s.active_workspace_id = id;
            Ok(())
        })
    }
    pub fn create_folder(&mut self, workspace_id: u64, parent_id: Option<u64>, title: &str) -> Result<u64> {
        let name = checked_title(title)?;
        self.edit(|s| {
            s.valid_parent(workspace_id, parent_id)?;
            if s.collections.len() >= MAX_FOLDERS { return Err(WorkspaceError::Invalid("Limite de dossiers atteinte")); }
            let id = s.allocate();
            s.collections.push(Collection { id, workspace_id, parent_id, name, order: id, appearance: Appearance { icon: "folder".into(), ..Default::default() } });
            Ok(id)
        })
    }
    pub fn rename_folder(&mut self, id: u64, title: &str) -> Result<()> {
        let title = checked_title(title)?;
        self.edit(|s| {
            s.collections.iter_mut().find(|f| f.id == id)
                .ok_or(WorkspaceError::NotFound("Dossier introuvable"))?.name = title;
            Ok(())
        })
    }
    /// Removing a folder lifts its children into its parent, preserving entries.
    pub fn remove_folder(&mut self, id: u64) -> Result<()> {
        self.edit(|s| {
            let f = s.folder(id).cloned().ok_or(WorkspaceError::NotFound("Dossier introuvable"))?;
            for child in &mut s.collections {
                if child.parent_id == Some(id) { child.parent_id = f.parent_id; }
            }
            for entry in &mut s.entries {
                if entry.folder_id == Some(id) { entry.folder_id = f.parent_id; }
            }
            s.collections.retain(|c| c.id != id);
            Ok(())
        })
    }
    pub fn move_folder(&mut self, id: u64, workspace_id: u64, parent_id: Option<u64>) -> Result<()> {
        self.edit(|s| {
            let folder = s.folder(id).cloned().ok_or(WorkspaceError::NotFound("Dossier introuvable"))?;
            if folder.workspace_id != workspace_id { return Err(WorkspaceError::Invalid("Un dossier ne se déplace que dans son espace")); }
            s.valid_parent(workspace_id, parent_id)?;
            s.collections.iter_mut().find(|f| f.id == id).unwrap().parent_id = parent_id;
            Ok(())
        })
    }
    pub fn save_resource(&mut self, workspace_id: u64, url: &str, title: &str, folder_id: Option<u64>) -> Result<u64> {
        let url = checked_url(url)?;
        let title = checked_title(title)?;
        self.edit(|s| {
            s.valid_parent(workspace_id, folder_id)?;
            if s.resources.len() >= MAX_RESOURCES || s.entries.len() >= MAX_ENTRIES {
                return Err(WorkspaceError::Invalid("Limite de ressources atteinte"));
            }
            let rid = s.resources.iter().find(|r| r.url == url).map(|r| r.id).unwrap_or_else(|| {
                let id = s.allocate();
                s.resources.push(SavedResource { id, url, title, notes: String::new(), tags: Vec::new(), favorite: false, appearance: Appearance { icon: "globe".into(), ..Default::default() } });
                id
            });
            if let Some(existing) = s.entries.iter().find(|e| e.workspace_id == workspace_id && e.resource_id == rid) {
                return Ok(existing.id);
            }
            let id = s.allocate();
            s.entries.push(WorkspaceEntry { id, workspace_id, resource_id: rid, folder_id, pinned: false, order: id });
            Ok(id)
        })
    }
    pub fn remove_entry(&mut self, id: u64) -> Result<()> {
        self.edit(|s| {
            let n = s.entries.len();
            s.entries.retain(|e| e.id != id);
            if s.entries.len() == n { return Err(WorkspaceError::NotFound("Entrée introuvable")); }
            let used: HashSet<_> = s.entries.iter().map(|e| e.resource_id).collect();
            s.resources.retain(|r| used.contains(&r.id) || r.favorite);
            Ok(())
        })
    }
    pub fn move_entry(&mut self, id: u64, workspace_id: u64, folder_id: Option<u64>) -> Result<()> {
        self.edit(|s| {
            s.valid_parent(workspace_id, folder_id)?;
            let e = s.entries.iter_mut().find(|e| e.id == id).ok_or(WorkspaceError::NotFound("Entrée introuvable"))?;
            e.workspace_id = workspace_id;
            e.folder_id = folder_id;
            Ok(())
        })
    }
    pub fn reorder_entry(&mut self, id: u64, direction: i32) -> Result<()> {
        self.edit(|s| {
            let entry = s.entry(id).cloned().ok_or(WorkspaceError::NotFound("Entrée introuvable"))?;
            let mut ids: Vec<_> = s.entries.iter().filter(|e| e.workspace_id == entry.workspace_id && e.folder_id == entry.folder_id && e.pinned == entry.pinned).map(|e| (e.order, e.id)).collect();
            ids.sort();
            let i = ids.iter().position(|(_, eid)| *eid == id).unwrap();
            let next = i as i32 + direction.signum();
            if next < 0 || next >= ids.len() as i32 { return Ok(()); }
            let other = ids[next as usize].1;
            // A strict total ordering is established on the affected siblings.
            ids.swap(i, next as usize);
            for (position, (_, eid)) in ids.into_iter().enumerate() {
                s.entries.iter_mut().find(|e| e.id == eid).unwrap().order = position as u64;
            }
            debug_assert_ne!(other, id);
            Ok(())
        })
    }
    pub fn set_pin(&mut self, id: u64, pinned: bool) -> Result<()> {
        self.edit(|s| { s.entries.iter_mut().find(|e| e.id == id).ok_or(WorkspaceError::NotFound("Entrée introuvable"))?.pinned = pinned; Ok(()) })
    }
    pub fn set_favorite(&mut self, resource_id: u64, favorite: bool) -> Result<()> {
        self.edit(|s| { s.resources.iter_mut().find(|r| r.id == resource_id).ok_or(WorkspaceError::NotFound("Ressource introuvable"))?.favorite = favorite; Ok(()) })
    }
    pub fn edit_resource(&mut self, id: u64, title: &str, notes: &str, tags: Vec<String>, appearance: Appearance) -> Result<()> {
        let title = checked_title(title)?;
        if notes.len() > 2000 { return Err(WorkspaceError::Invalid("Note trop longue")); }
        let tags = checked_tags(tags)?;
        checked_appearance(&appearance)?;
        self.edit(|s| {
            let r = s.resources.iter_mut().find(|r| r.id == id).ok_or(WorkspaceError::NotFound("Ressource introuvable"))?;
            r.title = title;
            r.notes = notes.to_owned();
            r.tags = tags;
            r.appearance = appearance;
            Ok(())
        })
    }
    pub fn set_appearance(&mut self, id: u64, appearance: Appearance) -> Result<()> {
        checked_appearance(&appearance)?;
        self.edit(|s| {
            if let Some(w) = s.workspaces.iter_mut().find(|w| w.id == id) { w.appearance = appearance; return Ok(()); }
            if let Some(f) = s.collections.iter_mut().find(|f| f.id == id) { f.appearance = appearance; return Ok(()); }
            Err(WorkspaceError::NotFound("Espace ou dossier introuvable"))
        })
    }
    pub fn reorder_workspace(&mut self, id: u64, direction: i32) -> Result<()> {
        self.edit(|s| {
            let mut ids: Vec<_> = s.workspaces.iter().map(|w| (w.order, w.id)).collect();
            ids.sort();
            let i = ids.iter().position(|(_, wid)| *wid == id).ok_or(WorkspaceError::NotFound("Espace introuvable"))?;
            let next = i as i32 + direction.signum();
            if next < 0 || next >= ids.len() as i32 { return Ok(()); }
            ids.swap(i, next as usize);
            for (position, (_, wid)) in ids.into_iter().enumerate() {
                s.workspaces.iter_mut().find(|w| w.id == wid).unwrap().order = position as u64;
            }
            Ok(())
        })
    }
    pub fn folders_depth_first(&self, workspace_id: u64) -> Vec<(&Collection, usize)> {
        fn visit<'a>(s: &'a WorkspaceStore, parent: Option<u64>, ws: u64, depth: usize, out: &mut Vec<(&'a Collection, usize)>) {
            let mut children: Vec<_> = s.collections.iter().filter(|f| f.workspace_id == ws && f.parent_id == parent).collect();
            children.sort_by_key(|f| (f.order, f.id));
            for child in children { out.push((child, depth)); visit(s, Some(child.id), ws, depth + 1, out); }
        }
        let mut result = Vec::new();
        visit(self, None, workspace_id, 0, &mut result);
        result
    }
    /// Derived favorites; never duplicates a resource or writes an extra collection.
    pub fn favorites(&self) -> Vec<&SavedResource> {
        self.resources.iter().filter(|r| r.favorite).collect()
    }
}

pub struct WorkspaceDisk { pub path: PathBuf }
impl WorkspaceDisk {
    pub fn new(path: PathBuf) -> Self { Self { path } }
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("aster-browser/workspaces-v1.json")
    }
    pub fn read(&self) -> Result<WorkspaceStore> {
        if fs::symlink_metadata(&self.path)?.file_type().is_symlink() {
            return Err(WorkspaceError::Invalid("Lien symbolique refusé pour la sauvegarde"));
        }
        let mut file = fs::File::open(&self.path)?;
        if file.metadata()?.len() > MAX_FILE_BYTES { return Err(WorkspaceError::Invalid("Sauvegarde trop volumineuse")); }
        let mut content = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut content)?;
        if content.len() as u64 > MAX_FILE_BYTES { return Err(WorkspaceError::Invalid("Sauvegarde trop volumineuse")); }
        let store: WorkspaceStore = serde_json::from_slice(&content)?;
        store.validate()?;
        Ok(store)
    }
    pub fn load_or_default(&self) -> Result<WorkspaceStore> {
        match self.read() {
            Err(WorkspaceError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(WorkspaceStore::default()),
            other => other,
        }
    }
    pub fn write(&self, store: &WorkspaceStore) -> Result<()> {
        store.validate()?;
        let content = serde_json::to_vec_pretty(store)?;
        if content.len() as u64 > MAX_FILE_BYTES { return Err(WorkspaceError::Invalid("Sauvegarde trop volumineuse")); }
        let parent = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        if self.path.exists() && fs::symlink_metadata(&self.path)?.file_type().is_symlink() {
            return Err(WorkspaceError::Invalid("Lien symbolique refusé pour la sauvegarde"));
        }
        // Unique temporary names prevent a crash from blocking later saves.
        // create_new also refuses to overwrite an in-flight temporary file.
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)
            .unwrap_or_default().as_nanos();
        let temp = self.path.with_extension(format!("json.tmp.{}-{nonce}", std::process::id()));
        let write_result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(&content)?;
            file.sync_all()?;
            if self.path.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
                }
                // Preserve a recovery copy before atomically switching snapshots.
                let backup = self.path.with_extension("json.bak");
                let backup_temp = self.path.with_extension(format!("json.bak.tmp.{}-{nonce}", std::process::id()));
                fs::copy(&self.path, &backup_temp)?;
                OpenOptions::new().read(true).open(&backup_temp)?.sync_all()?;
                if let Err(error) = fs::rename(&backup_temp, &backup) {
                    let _ = fs::remove_file(&backup_temp);
                    return Err(WorkspaceError::Io(error));
                }
            }
            fs::rename(&temp, &self.path)?;
            if let Ok(directory) = OpenOptions::new().read(true).open(parent) { let _ = directory.sync_all(); }
            Ok(())
        })();
        if write_result.is_err() { let _ = fs::remove_file(&temp); }
        write_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> WorkspaceStore {
        let mut s = WorkspaceStore::default();
        let w = s.active_workspace();
        s.save_resource(w, "https://example.org/", "Exemple", None).unwrap();
        s
    }
    #[test]
    fn folders_and_cross_workspace_references() {
        let mut s = example();
        let first = s.entries[0].id;
        let original_rid = s.entries[0].resource_id;
        let second = s.create_workspace("Studio").unwrap();
        let folder = s.create_folder(second, None, "Production").unwrap();
        let nested = s.create_folder(second, Some(folder), "Références").unwrap();
        let id = s.save_resource(second, "https://example.org/", "Dupliqué", Some(nested)).unwrap();
        assert_eq!(s.entry(id).unwrap().resource_id, original_rid);
        assert_eq!(s.resources.len(), 1);
        assert!(s.move_folder(folder, second, Some(nested)).is_err());
        assert!(s.move_entry(first, second, Some(999)).is_err());
        assert_eq!(s.entry(first).unwrap().workspace_id, 1);
        s.remove_folder(folder).unwrap();
        assert_eq!(s.folder(nested).unwrap().parent_id, None);
        s.remove_entry(id).unwrap();
        assert_eq!(s.resources.len(), 1);
        s.validate().unwrap();
    }
    #[test]
    fn favorites_and_deletion_are_non_destructive() {
        let mut s = example();
        let r = s.entries[0].resource_id;
        s.set_favorite(r, true).unwrap();
        let second = s.create_workspace("Projet").unwrap();
        let new = s.save_resource(second, "https://example.net", "Site", None).unwrap();
        s.remove_workspace(second).unwrap();
        assert_eq!(s.entry(new).unwrap().workspace_id, 1);
        assert_eq!(s.favorites().len(), 1);
        assert_eq!(s.workspaces.len(), 1);
        assert!(s.remove_workspace(1).is_err());
    }
    #[test]
    fn validation_rejects_corrupt_cyclic_and_unknown_versions() {
        let mut s = example();
        let folder = s.create_folder(1, None, "Racine").unwrap();
        s.collections[0].parent_id = Some(folder);
        assert!(s.validate().is_err());
        s.collections[0].parent_id = None;
        s.version = 42;
        assert!(s.validate().is_err());
        s.version = 1;
        assert!(s.save_resource(1, "javascript:alert(1)", "Danger", None).is_err());
    }
    #[test]
    fn serialization_is_validated_and_atomic() {
        let path = std::env::temp_dir().join(format!("aster-spaces-test-{}-{}.json", std::process::id(), 598731));
        let disk = WorkspaceDisk::new(path.clone());
        let mut s = example();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json.bak"));
        disk.write(&s).unwrap();
        assert_eq!(disk.read().unwrap(), s);
        s.create_workspace("Suivant").unwrap();
        disk.write(&s).unwrap();
        assert_eq!(disk.read().unwrap(), s);
        assert!(path.with_extension("json.bak").exists());
        fs::write(&path, b"{corrupt").unwrap();
        assert!(disk.load_or_default().is_err());
        assert_eq!(serde_json::from_slice::<WorkspaceStore>(&fs::read(path.with_extension("json.bak")).unwrap()).unwrap().workspaces.len(), 1);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json.bak"));
    }
    #[test]
    fn reorder_and_edit_are_validated() {
        let mut s = example();
        let x = s.entries[0].id;
        let b = s.save_resource(1, "https://example.net", "Other", None).unwrap();
        s.reorder_entry(b, -1).unwrap();
        assert!(s.entry(b).unwrap().order < s.entry(x).unwrap().order);
        let rid = s.entry(x).unwrap().resource_id;
        s.edit_resource(rid, "Titre", "note", vec!["Rust".into(), "rust".into()], Appearance::default()).unwrap();
        assert_eq!(s.resource(rid).unwrap().tags, vec!["rust".to_string()]);
        assert!(s.set_appearance(1, Appearance { color: "red".into(), icon: "x".into() }).is_err());
    }
}
