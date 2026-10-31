use crate::{live::Field, project::Workspace};
use dioxus_native::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecord {
    id: u32,
    config: Workspace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    version: u32,
    active: u32,
    workspaces: Vec<WorkspaceRecord>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            active: 1,
            workspaces: vec![WorkspaceRecord {
                id: 1,
                config: Workspace::default(),
            }],
        }
    }
}

impl Catalog {
    pub fn active_id(&self) -> u32 {
        self.active
    }
    fn current(&self) -> &Workspace {
        &self
            .workspaces
            .iter()
            .find(|w| w.id == self.active)
            .expect("active workspace exists")
            .config
    }

    fn update(&mut self, config: Workspace) {
        self.workspaces
            .iter_mut()
            .find(|w| w.id == self.active)
            .expect("active workspace exists")
            .config = config;
    }

    fn add(&mut self, config: Workspace) {
        let id = (1..=u32::MAX)
            .find(|id| !self.workspaces.iter().any(|w| w.id == *id))
            .expect("workspace IDs exhausted");
        self.workspaces.push(WorkspaceRecord { id, config });
        self.active = id;
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let catalog: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let mut ids = std::collections::HashSet::new();
        if catalog.version != 1
            || catalog.workspaces.is_empty()
            || !catalog.workspaces.iter().any(|w| w.id == catalog.active)
            || catalog
                .workspaces
                .iter()
                .any(|w| w.id == 0 || !ids.insert(w.id) || w.config.version != 1)
        {
            return Err("Invalid workspace catalog".into());
        }
        Ok(catalog)
    }
}

pub fn use_workspaces() -> (Signal<Workspace>, Signal<Catalog>, Signal<String>) {
    let loaded = use_hook(|| load().map(|c| c.unwrap_or_default()));
    let saving_enabled = loaded.is_ok();
    let mut catalog = use_signal(|| loaded.clone().unwrap_or_default());
    let project = use_signal(|| catalog.peek().current().clone());
    let mut status = use_signal(|| match &loaded {
        Ok(_) => "Saved locally".to_owned(),
        Err(e) => format!("Local storage unavailable: {e}. Export to keep changes."),
    });
    // Read only the active configuration here. Switching updates both signals
    // in the same event; changes are copied into that workspace before saving.
    use_effect(move || {
        let config = project.read().clone();
        if *catalog.peek().current() != config {
            catalog.write().update(config);
        }
    });
    use_effect(move || {
        let snapshot = catalog.read().clone();
        if saving_enabled {
            status.set(match save(&snapshot) {
                Ok(()) => "Saved locally".into(),
                Err(e) => format!("Could not save locally: {e}. Export to keep changes."),
            });
        }
    });
    (project, catalog, status)
}

#[component]
pub fn Workspaces(
    mut project: Signal<Workspace>,
    mut catalog: Signal<Catalog>,
    storage_status: Signal<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut notice = use_signal(String::new);
    rsx! {div {class:"workspace-controls",
        div {class:"workspace-toolbar",
            button {class:"workspace-switch",onclick:move |_|open.set(!open()),"{project.read().name} ▾"}
            span {class:"form-help",role:"status","{storage_status}"}
        }
        if open() {section {class:"workspace-manager",
            div {class:"panel-heading",div {h2 {"Workspace settings"}p {"Sources, scenes and both stream formats belong to this workspace."}}button {onclick:move |_|open.set(false),"Close"}}
            div {class:"padded",
                Field {label:"Workspace name",value:project.read().name.clone(),change:move |v|project.write().name=v}
                div {class:"workspace-list",role:"group","aria-label":"Workspaces",
                    for workspace in catalog.read().workspaces.clone() {
                        button {class:if workspace.id==catalog.read().active {"selected"}else{""},onclick:move |_| {
                            let mut next=catalog.peek().clone();next.update(project.peek().clone());next.active=workspace.id;
                            project.set(next.current().clone());catalog.set(next);notice.set(String::new());
                        },"{workspace.config.name}"}
                    }
                }
                div {class:"row",
                    button {class:"primary",onclick:move |_| {
                        let mut next=catalog.peek().clone();next.update(project.peek().clone());
                        let config=Workspace { name:format!("Workspace {}",next.workspaces.len()+1), ..Workspace::default() };
                        next.add(config.clone());project.set(config);catalog.set(next);notice.set("New workspace created".into());
                    },"+ New workspace"}
                    button {disabled:busy(),onclick:move |_| {
                        match project.read().json() {
                            Err(e)=>notice.set(e),
                            Ok(json)=>{busy.set(true);spawn(async move {
                                if let Some(file)=rfd::AsyncFileDialog::new().set_file_name("acheflow-workspace.json").add_filter("JSON",&["json"]).save_file().await {
                                    notice.set(match file.write(json.as_bytes()).await {Ok(())=>"Workspace exported".into(),Err(e)=>format!("Export failed: {e}")});
                                }busy.set(false);
                            });}
                        }
                    },"Export workspace"}
                    button {disabled:busy(),onclick:move |_| {busy.set(true);spawn(async move {
                        if let Some(file)=rfd::AsyncFileDialog::new().add_filter("JSON",&["json"]).pick_file().await {
                            match Workspace::from_json(&file.read().await) {
                                Ok(mut config)=>{config.id=crate::project::new_token();let mut next=catalog.peek().clone();next.update(project.peek().clone());next.add(config.clone());project.set(config);catalog.set(next);notice.set("Workspace added from file".into());},
                                Err(e)=>notice.set(format!("File rejected: {e}")),
                            }
                        }busy.set(false);
                    });},"Add from file"}
                }
                p {class:"form-help","Workspace files and local storage include RTMP keys."}
                if !notice.read().is_empty(){p {role:"status","{notice}"}}
            }
        }}
    }}
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or("No browser window")?
        .local_storage()
        .map_err(|e| format!("{e:?}"))?
        .ok_or("Storage is disabled".into())
}
#[cfg(target_arch = "wasm32")]
fn load() -> Result<Option<Catalog>, String> {
    storage()?
        .get_item("acheflow.workspaces.v1")
        .map_err(|e| format!("{e:?}"))?
        .map(|s| Catalog::decode(s.as_bytes()))
        .transpose()
}
#[cfg(target_arch = "wasm32")]
fn save(catalog: &Catalog) -> Result<(), String> {
    let json = serde_json::to_string(catalog).map_err(|e| e.to_string())?;
    storage()?
        .set_item("acheflow.workspaces.v1", &json)
        .map_err(|e| format!("{e:?}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn storage_path() -> Result<std::path::PathBuf, String> {
    if let Some(path) = std::env::var_os("ACHEFLOW_WORKSPACE_FILE") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    Ok(std::path::PathBuf::from(home).join(".acheflow/workspaces.json"))
}
#[cfg(not(target_arch = "wasm32"))]
fn load() -> Result<Option<Catalog>, String> {
    match std::fs::read(storage_path()?) {
        Ok(bytes) => Catalog::decode(&bytes).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn save(catalog: &Catalog) -> Result<(), String> {
    use std::io::Write;
    let path = storage_path()?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec(catalog).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    file.write_all(&json)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creating_and_importing_keep_workspace_configuration_isolated() {
        let mut c = Catalog::default();
        let mut first = c.current().clone();
        first.add_source();
        first.add_scene(crate::project::Orientation::Vertical);
        c.update(first.clone());
        c.add(Workspace::default());
        assert!(c.current().sources.is_empty());
        c.add(Workspace::from_json(first.json().unwrap().as_bytes()).unwrap());
        assert_eq!(c.workspaces.len(), 3);
        assert_eq!(c.current(), &first);
        c.active = 2;
        assert!(c.current().scenes.is_empty());
        c.active = 1;
        assert_eq!(c.current(), &first);
        assert_eq!(
            Catalog::decode(&serde_json::to_vec(&c).unwrap()).unwrap(),
            c
        );
    }
    #[test]
    fn invalid_catalog_selection_and_duplicate_ids_are_rejected() {
        let mut c = Catalog {
            active: 9,
            ..Catalog::default()
        };
        assert!(Catalog::decode(&serde_json::to_vec(&c).unwrap()).is_err());
        c.active = 1;
        c.workspaces.push(c.workspaces[0].clone());
        assert!(Catalog::decode(&serde_json::to_vec(&c).unwrap()).is_err());
    }
}
