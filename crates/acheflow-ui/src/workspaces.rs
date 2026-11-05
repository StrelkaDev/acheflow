use crate::{live::Field, project::Workspace};
use acheflow_client::catalog::ClientCatalog;
use dioxus_native::prelude::*;
use serde::{Deserialize, Serialize};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRecord {
    id: u32,
    config: Workspace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Preferences {
    background_animation: bool,
    server: crate::server_settings::ServerPreferences,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            background_animation: true,
            server: Default::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default)]
    preferences: Preferences,
    version: u32,
    active: u32,
    workspaces: Vec<WorkspaceRecord>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            preferences: Preferences::default(),
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
    #[cfg(not(target_arch = "wasm32"))]
    pub fn server_preferences(&self) -> crate::server_settings::ServerPreferences {
        self.preferences.server.clone()
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_server_preferences(
        &mut self,
        preferences: crate::server_settings::ServerPreferences,
    ) {
        self.preferences.server = preferences;
    }
    pub fn background_animation(&self) -> bool {
        self.preferences.background_animation
    }
    pub fn set_background_animation(&mut self, enabled: bool) {
        self.preferences.background_animation = enabled;
    }
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

    fn add(&mut self, mut config: Workspace) {
        config.migrate_spatial();
        let id = (1..=u32::MAX)
            .find(|id| !self.workspaces.iter().any(|w| w.id == *id))
            .expect("workspace IDs exhausted");
        self.workspaces.push(WorkspaceRecord { id, config });
        self.active = id;
    }

    fn decode(bytes: &[u8]) -> Result<Self, String> {
        let mut catalog: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
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
        catalog.preferences.server.validate()?;
        for record in &mut catalog.workspaces {
            record.config.migrate_spatial();
        }
        Ok(catalog)
    }
}

pub fn use_workspaces() -> (Signal<Workspace>, Signal<Catalog>, Signal<String>) {
    let client_catalog = use_hook(|| Rc::new(ClientCatalog::discover()));
    use_context_provider(|| client_catalog.clone());
    let loaded = use_hook(|| {
        load().map(|c| {
            let mut c = c.unwrap_or_default();
            for record in &mut c.workspaces {
                record
                    .config
                    .migrate_client_extensions(client_catalog.installed.keys().map(String::as_str));
            }
            c
        })
    });
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
pub fn WorkspaceSwitcher(project: Signal<Workspace>, mut open: Signal<bool>) -> Element {
    let name = project.read().name.clone();
    rsx! {
        button {class:"workspace-switch",title:"{name}","aria-label":"Choose workspace: {name}","aria-expanded":open().to_string(),"aria-controls":"workspace-manager",onclick:move |_|open.set(!open()),
            span {class:"workspace-icon","A"}
            span {class:"workspace-switch-name","{name}"}
            span {class:"workspace-switch-caret","▾"}
        }
    }
}

#[component]
pub fn Workspaces(
    mut project: Signal<Workspace>,
    mut catalog: Signal<Catalog>,
    storage_status: Signal<String>,
    mut open: Signal<bool>,
) -> Element {
    let client_catalog = use_context::<Rc<ClientCatalog>>();
    let mut busy = use_signal(|| false);
    let mut notice = use_signal(String::new);
    let mut creating = use_signal(|| false);
    let server = crate::media::server_url(&project.read(), "").unwrap_or_else(|e| e);
    rsx! {div {class:"workspace-controls",
        if open() {section {class:"workspace-manager",id:"workspace-manager",
            div {class:"panel-heading",div {h2 {"Workspace settings"}p {"Sources, scenes and both stream formats belong to this workspace."}}button {onclick:move |_|open.set(false),"Close"}}
            div {class:"padded",
                Field {label:"Workspace name",value:project.read().name.clone(),change:move |v|project.write().name=v}
                p {class:"form-help","Server: {server}"}
                div {class:"workspace-list",role:"group","aria-label":"Workspaces",
                    for workspace in catalog.read().workspaces.clone() {
                        button {class:if workspace.id==catalog.read().active {"selected"}else{""},onclick:move |_| {
                            let mut next=catalog.peek().clone();next.update(project.peek().clone());next.active=workspace.id;
                            project.set(next.current().clone());catalog.set(next);notice.set(String::new());
                        },"{workspace.config.name}"}
                    }
                }
                div {class:"row",
                    button {class:"primary",onclick:move |_|creating.set(true),"+ New workspace"}
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
                    button {disabled:busy(),onclick:move |_| {busy.set(true);let client_catalog=client_catalog.clone();spawn(async move {
                        if let Some(file)=rfd::AsyncFileDialog::new().add_filter("JSON",&["json"]).pick_file().await {
                            match Workspace::from_json(&file.read().await) {
                                Ok(mut config)=>{config.migrate_client_extensions(client_catalog.installed.keys().map(String::as_str));config.id=crate::project::new_token();let mut next=catalog.peek().clone();next.update(project.peek().clone());next.add(config.clone());project.set(config);catalog.set(next);notice.set("Workspace added from file".into());},
                                Err(e)=>notice.set(format!("File rejected: {e}")),
                            }
                        }busy.set(false);
                    });},"Add from file"}
                }
                if creating() {
                    WorkspaceCreation {
                        name:format!("Workspace {}",catalog.read().workspaces.len()+1),
                        server:server.clone(),
                        cancel:move |_|creating.set(false),
                        create:move |config:Workspace| {
                            let mut next=catalog.peek().clone();next.update(project.peek().clone());
                            next.add(config.clone());project.set(config);catalog.set(next);
                            creating.set(false);notice.set("Workspace created. Connecting to its server…".into());
                        }
                    }
                }
                p {class:"form-help",role:"status","{storage_status}"}
                p {class:"form-help","Workspace files and local storage include RTMP keys."}
                if !notice.read().is_empty(){p {role:"status","{notice}"}}
            }
        }}
    }}
}

fn new_workspace(name: &str, server: &str) -> Result<Workspace, String> {
    if name.trim().is_empty() {
        return Err("Workspace name is required.".into());
    }
    Ok(Workspace {
        name: name.trim().into(),
        server_url: acheflow_media_api::workspace::normalize_server_url(server)?,
        ..Workspace::default()
    })
}

#[component]
fn WorkspaceCreation(
    name: String,
    server: String,
    create: EventHandler<Workspace>,
    cancel: EventHandler<()>,
) -> Element {
    let mut name = use_signal(|| name);
    let mut server = use_signal(|| server);
    let mut error = use_signal(|| None::<String>);
    let choose_server = crate::media::can_choose_server();
    let mut submit = move || match new_workspace(&name.peek(), &server.peek()) {
        Ok(config) => create.call(config),
        Err(message) => error.set(Some(message)),
    };
    rsx! {form {class:"workspace-create",onsubmit:move |event|event.prevent_default(),onkeydown:move |event| {
        if event.key()==Key::Named(NamedKey::Enter) {event.prevent_default();submit();}
    },
        h3 {"New workspace"}
        div {class:"form-grid",
            label {class:"field",span {"Workspace name"}input {r#type:"text","aria-label":"New workspace name",value:name.peek().clone(),oninput:move |event|name.set(event.value())}}
            div {class:"field",span {"Server address"}
                if choose_server {input {r#type:"text","aria-label":"Server address",placeholder:"http://127.0.0.1:21295",value:server.peek().clone(),oninput:move |event|server.set(event.value())}}
                else {div {class:"readonly-field",role:"textbox","aria-label":"Server address","aria-readonly":"true",tabindex:"0","{server.peek()}"}}
            }
        }
        p {class:"form-help",if choose_server {"Enter the server's HTTP(S) address. API requests, previews and RTMP connection details use this server."}else{"This workspace uses the server hosting this interface. RTMP connection details are discovered automatically."}}
        if let Some(message)=error(){p {class:"field-error",role:"alert","{message}"}}
        div {class:"row",button {r#type:"button",class:"primary",onclick:move |_|submit(),"Create workspace"}button {r#type:"button",onclick:move |_|cancel.call(()),"Cancel"}}
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
pub(crate) fn storage_path() -> Result<std::path::PathBuf, String> {
    if let Some(path) = std::env::var_os("ACHEFLOW_WORKSPACE_FILE") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    Ok(std::path::PathBuf::from(home).join(".acheflow/workspaces.json"))
}
#[cfg(not(target_arch = "wasm32"))]
fn load() -> Result<Option<Catalog>, String> {
    load_at(&storage_path()?)
}
#[cfg(not(target_arch = "wasm32"))]
fn load_at(path: &std::path::Path) -> Result<Option<Catalog>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Catalog::decode(&bytes).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn save(catalog: &Catalog) -> Result<(), String> {
    save_at(&storage_path()?, catalog)
}
#[cfg(not(target_arch = "wasm32"))]
fn save_at(path: &std::path::Path, catalog: &Catalog) -> Result<(), String> {
    use std::io::Write;
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
        first.server_url = "https://first.example:8443".into();
        first.extensions.entry("sample".into()).or_default().enabled = true;
        first
            .extensions
            .entry("twitch".into())
            .or_default()
            .config
            .insert("client_id".into(), "firstpublicapp".into());
        first.add_source();
        first.add_scene(crate::project::Orientation::Vertical);
        first.migrate_spatial();
        c.update(first.clone());
        c.add(new_workspace("Second", "http://second.example:21296/").unwrap());
        assert!(c.current().sources.is_empty());
        assert!(c.current().extensions.is_empty());
        c.add(Workspace::from_json(first.json().unwrap().as_bytes()).unwrap());
        assert_eq!(c.workspaces.len(), 3);
        assert_eq!(c.current(), &first);
        c.active = 2;
        assert!(c.current().scenes.is_empty());
        assert_eq!(c.current().server_url, "http://second.example:21296");
        c.active = 1;
        assert_eq!(c.current(), &first);
        assert_eq!(
            Catalog::decode(&serde_json::to_vec(&c).unwrap()).unwrap(),
            c
        );
    }
    #[test]
    fn creation_requires_a_name_and_an_explicit_server_address() {
        assert!(new_workspace("Test", "").is_err());
        assert!(new_workspace(" ", "http://localhost:21295").is_err());
        assert!(new_workspace("Test", "rtmp://localhost/live").is_err());
        let workspace = new_workspace(" Test ", " https://host:8443/path/ ").unwrap();
        assert_eq!(workspace.name, "Test");
        assert_eq!(workspace.server_url, "https://host:8443/path");
        assert!(workspace.sources.is_empty());
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod disk_tests {
    use super::*;
    use acheflow_extension_api::client_execution::{Capability, CapabilityGrant};
    #[test]
    fn native_catalog_restores_selected_workspace_public_settings_and_policy_from_disk() {
        let root = std::env::temp_dir().join(format!(
            "acheflow-workspace-storage-{}",
            crate::project::new_token()
        ));
        let path = root.join("nested/workspaces.json");
        assert!(load_at(&path).unwrap().is_none());
        let mut catalog = Catalog::default();
        catalog.set_background_animation(false);
        let mut first = catalog.current().clone();
        first.server_url = "https://first.example".into();
        first.edit_revision = 7;
        first
            .extensions
            .entry("fixture".into())
            .or_default()
            .enabled = true;
        first
            .extensions
            .get_mut("fixture")
            .unwrap()
            .config
            .insert("application".into(), "public-app".into());
        first.client_extensions.insert(
            "local-provider".into(),
            crate::project::ExtensionSettings {
                enabled: true,
                config: std::collections::BTreeMap::from([(
                    "application".into(),
                    "local-native-app".into(),
                )]),
            },
        );
        first.client_capabilities = vec![CapabilityGrant {
            provider: "fixture".into(),
            account_id: "bot".into(),
            channel_id: "channel".into(),
            capabilities: [Capability::ChatRead].into(),
        }];
        catalog.update(first.clone());
        catalog.add(new_workspace("Second", "https://second.example").unwrap());
        save_at(&path, &catalog).unwrap();
        let mut reopened = load_at(&path).unwrap().unwrap();
        assert_eq!(reopened, catalog);
        assert!(!reopened.background_animation());
        assert_eq!(reopened.active_id(), 2);
        assert!(reopened.current().extensions.is_empty());
        assert!(reopened.current().client_capabilities.is_empty());
        reopened.active = 1;
        assert_eq!(reopened.current(), &first);
        let imported = Workspace::from_json(first.json().unwrap().as_bytes()).unwrap();
        assert_eq!(imported.client_capabilities, first.client_capabilities);
        assert_eq!(imported.extensions, first.extensions);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, b"{broken").unwrap();
        assert!(
            load_at(&path).is_err(),
            "corrupt storage must not silently become an empty catalog"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    #[test]
    fn old_preferences_default_to_opt_in_server_lifecycle() {
        let mut value = serde_json::to_value(Catalog::default()).unwrap();
        value["preferences"] = serde_json::json!({"background_animation": false});
        let restored = Catalog::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(!restored.background_animation());
        assert_eq!(restored.preferences.server, Default::default());
        for port in [0, 1935] {
            value["preferences"]["server"] = serde_json::json!({"http_port":port});
            assert!(Catalog::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn server_preferences_restore_from_disk_and_stay_out_of_workspace_exports() {
        let path =
            std::env::temp_dir().join(format!("af-server-preferences-{}.json", std::process::id()));
        let mut catalog = Catalog::default();
        let prefs = crate::server_settings::ServerPreferences {
            start_with_client: true,
            keep_running_on_exit: true,
            http_port: 22345,
            rtmp_port: 22346,
        };
        catalog.set_server_preferences(prefs.clone());
        catalog.add(new_workspace("Remote", "https://remote.example").unwrap());
        save_at(&path, &catalog).unwrap();
        let restored = load_at(&path).unwrap().unwrap();
        assert_eq!(restored.server_preferences(), prefs);
        let workspace = restored.current().json().unwrap();
        assert!(!workspace.contains("start_with_client"));
        assert!(!workspace.contains("keep_running_on_exit"));
        assert_eq!(restored.current().server_url, "https://remote.example");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn old_catalogs_get_default_appearance_and_switching_keeps_preferences_global() {
        let mut legacy = serde_json::to_value(Catalog::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("preferences");
        let mut catalog = Catalog::decode(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(catalog.background_animation());
        catalog.set_background_animation(false);
        catalog.add(new_workspace("Second", "https://server.example").unwrap());
        assert!(!catalog.background_animation());
        catalog.active = 1;
        let restored = Catalog::decode(&serde_json::to_vec(&catalog).unwrap()).unwrap();
        assert!(!restored.background_animation());
        assert!(
            !restored
                .current()
                .json()
                .unwrap()
                .contains("background_animation")
        );
    }
}
