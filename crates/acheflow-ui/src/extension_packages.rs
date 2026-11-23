//! Review code and authority separately from installation and enabling a guest.
use crate::{control_client::ControlClient, server_access, server_settings::ActionButton};
use acheflow_extension_api::package::{InstallPackage, PackageReview};
use acheflow_media_api::access::Permission as P;
use dioxus_native::prelude::*;
#[component]
pub(crate) fn Packages(server: bool) -> Element {
    let mut client = use_context::<ControlClient>();
    let mut expanded = use_signal(|| false);
    let mut review = use_signal(|| None::<PackageReview>);
    let mut installed = use_signal(Vec::<PackageReview>::new);
    let mut notice = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut trusted = use_signal(|| false);
    let mut revision = use_signal(|| 0u64);
    let owner = use_memo(move || {
        let p = client.project.read();
        (p.server_url.clone(), server_access::identity())
    });
    use_resource(move || {
        let _ = revision();
        let owner = owner();
        let show = expanded();
        async move {
            if !show {
                return;
            }
            review.set(None);
            trusted.set(false);
            installed.write().clear();
            error.set(String::new());
            if server
                && ![P::ExtensionsUpload, P::ExtensionsInstall, P::ExtensionsRun]
                    .into_iter()
                    .any(server_access::allows)
            {
                return;
            }
            match list(server, client).await {
                Ok(values) if owner == owner_now(client) => installed.set(values),
                Err(message) if owner == owner_now(client) => error.set(message),
                _ => {}
            }
        }
    });
    let can_upload = !server || server_access::allows(P::ExtensionsUpload);
    let can_install = !server || server_access::allows(P::ExtensionsInstall);
    let can_run = !server || server_access::allows(P::ExtensionsRun);
    let supported = server || cfg!(not(target_arch = "wasm32"));
    if server && !can_upload && !can_install && !can_run {
        return rsx! {};
    }
    rsx! {section {class:"panel package-manager",div{class:"panel-heading",div{h2{"Extension packages"}p{class:"form-help",if server{"Install on this server. Installation leaves execution disabled."}else{if cfg!(target_arch="wasm32"){"Browser clients use their bundled client parts. Upload server packages in the Server tab."}else{"Install on this desktop. Restart the app after allowing execution."}}}}
        PackageAction{id:if server{"server-packages-expand"}else{"client-packages-expand"},label:(if expanded(){"Hide"}else{"Manage packages"}).to_owned(),disabled:busy(),onclick:move|_|expanded.set(!expanded())}}
        if expanded(){div{class:"padded",
            if !supported {p{class:"form-help","This browser uses bundled client extensions. You can upload server packages from the Server tab."}}
            if supported && can_upload {div{class:"row",PackageAction{id:if server{"server-package-choose"}else{"client-package-choose"},label:"Choose ZIP package".to_owned(),disabled:busy(),onclick:move|_|{
                if busy(){return;}let target=owner_now(client);busy.set(true);error.set(String::new());review.set(None);trusted.set(false);
                spawn(async move{
                    if let Some(file)=rfd::AsyncFileDialog::new().add_filter("Extension package",&["zip"]).pick_file().await {
                        let bytes=file.read().await;
                        if target==owner_now(client){match stage(server,client,bytes).await{Ok(value) if target==owner_now(client)=>review.set(Some(value)),Err(message) if target==owner_now(client)=>error.set(message),_=>{}}}
                    }
                    busy.set(false);
                });
            }}}}
            if let Some(package)=review(){
                div{class:"package-review",h3{"{package.name}"}p{class:"form-help","{package.id} · {package.digest}"}
                    p{"Capabilities: " {package.capabilities.join(", ")}}
                    p{"Actions: " {package.supported_actions.join(", ")}}
                    p{"Network access: " {package.services.join(", ")}}
                    if package.requires_authorization {p{class:"form-help","This part declares account authorization. Installing it does not connect an account."}}
                    if package.replaces.is_some(){p{class:"form-help","This replaces an installed package and retires its running instance."}}
                    label{class:"toggle-field",input{r#type:"checkbox",checked:trusted(),onchange:move|event|trusted.set(event.checked())}span{if server{"I trust this code and the declared access on this server."}else{"I trust this code and the declared access on this device."}}}
                    PackageAction{id:if server{"server-package-install"}else{"client-package-install"},label:"Install, leave disabled".to_owned(),disabled:busy()||!trusted()||!can_install,onclick:move|_|{
                        if busy()||!trusted(){return;}let target=owner_now(client);let package=package.clone();busy.set(true);error.set(String::new());
                        spawn(async move{let result=install(server,client,&package).await;if target==owner_now(client){match result {Ok(())=>{
                            if !server && let Some(setting)=client.project.write().client_extensions.get_mut(&package.id){setting.enabled=false;}
                            notice.set(if server{"Installed. Execution requires a separate permission below."}else{"Installed. Allow execution below, then restart this app."}.into());revision.with_mut(|v|*v+=1);
                        },Err(message)=>error.set(message)}}busy.set(false);});
                    }}
                    if !can_install {p{class:"form-help","Your device can upload this package but cannot install it."}}
                }
            }
            for package in installed(){div{class:"package-installed",strong{"{package.name}"}p{class:"form-help","{package.id} · {package.digest}"}
                if package.execution_allowed {p{class:"form-help","Execution allowed; workspace controls and grants still apply."}}
                else {PackageAction{id:format!("package-allow-{server}-{}",package.id),label:"Allow execution".to_owned(),disabled:busy()||!can_run,onclick:move|_|{
                    if busy(){return;}let target=owner_now(client);let package=package.clone();busy.set(true);error.set(String::new());
                    spawn(async move{let result=allow(server,client,&package).await;if target==owner_now(client){match result{Ok(())=>{notice.set(if server{"Execution allowed. Configure and enable this extension in the workspace."}else{"Execution allowed. Restart this app to load the client part."}.into());revision.with_mut(|v|*v+=1);},Err(message)=>error.set(message)}}busy.set(false);});
                }}}
            }}
            if !notice().is_empty(){p{class:"form-help",role:"status","{notice}"}}
            if !error().is_empty(){p{class:"field-error",role:"alert","{error}"}}
        }}
    }}
}
fn owner_now(client: ControlClient) -> (String, Option<String>) {
    let p = client.project.read();
    (p.server_url.clone(), server_access::identity())
}
async fn list(server: bool, client: ControlClient) -> Result<Vec<PackageReview>, String> {
    if server {
        let endpoint =
            crate::media::server_url(&client.project.read(), "/api/extensions/packages")?;
        return server_access::request(reqwest::Method::GET, &endpoint)?
            .send()
            .await
            .map_err(|_| "Package server is unavailable.")?
            .error_for_status()
            .map_err(|_| "Cannot list installed packages.")?
            .json()
            .await
            .map_err(|_| "Invalid package list.".into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let root = acheflow_client::package::client_root()?;
        if !root.exists() {
            return Ok(vec![]);
        }
        local_store(move |store| {
            store
                .installed()
                .map(|values| values.iter().map(|v| v.review().clone()).collect())
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(vec![])
    }
}
async fn stage(
    server: bool,
    client: ControlClient,
    bytes: Vec<u8>,
) -> Result<PackageReview, String> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Package exceeds the 8 MiB limit.".into());
    }
    if server {
        let endpoint =
            crate::media::server_url(&client.project.read(), "/api/extensions/packages/stage")?;
        return server_access::request(reqwest::Method::POST, &endpoint)?
            .header("content-type", "application/zip")
            .body(bytes)
            .send()
            .await
            .map_err(|_| "Package upload interrupted.")?
            .error_for_status()
            .map_err(|_| "Package rejected. Check its files, permissions, host part and format.")?
            .json()
            .await
            .map_err(|_| "Invalid package review.".into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        local_store(move |store| {
            store.stage(
                &bytes,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            )
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        Err("Install custom client packages in the desktop app.".into())
    }
}
async fn install(
    server: bool,
    client: ControlClient,
    package: &PackageReview,
) -> Result<(), String> {
    let request = InstallPackage {
        digest: package.digest.clone(),
        expected_current: package.replaces.clone(),
    };
    if server {
        return post(client, "install", &request).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let catalog = dioxus_native::prelude::try_consume_context::<
            std::rc::Rc<acheflow_client::catalog::ClientCatalog>,
        >();
        let base_ids: std::collections::BTreeSet<_> = catalog
            .map(|c| c.installed.keys().cloned().collect())
            .unwrap_or_default();
        local_store(move |store| {
            let prepared = store.prepare(&request.digest, request.expected_current.as_deref())?;
            let mut ids = base_ids;
            for package in store.installed()? {
                ids.insert(package.review().id.clone());
            }
            ids.insert(prepared.review().id.clone());
            if ids.len() > 8 {
                return Err("The client extension catalog is full.".into());
            }
            store.commit(&prepared)
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        Err("Client package installation requires the desktop app.".into())
    }
}
async fn allow(server: bool, client: ControlClient, package: &PackageReview) -> Result<(), String> {
    if server {
        return post(
            client,
            "allow",
            &InstallPackage {
                digest: package.digest.clone(),
                expected_current: None,
            },
        )
        .await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = package.id.clone();
        let digest = package.digest.clone();
        local_store(move |store| store.allow_execution(&id, &digest)).await
    }
    #[cfg(target_arch = "wasm32")]
    {
        Err("Client package activation requires the desktop app.".into())
    }
}
async fn post(client: ControlClient, action: &str, request: &InstallPackage) -> Result<(), String> {
    let endpoint = crate::media::server_url(
        &client.project.read(),
        &format!("/api/extensions/packages/{action}"),
    )?;
    let response = server_access::request(reqwest::Method::POST, &endpoint)?
        .json(request)
        .send()
        .await
        .map_err(|_| "Package request interrupted.")?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err("Package request refused. Refresh the review and check your device permissions.".into())
    }
}

#[component]
fn PackageAction(
    id: String,
    label: String,
    #[props(default)] disabled: bool,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {ActionButton{id,disabled,onclick,"{label}"}}
}

#[cfg(not(target_arch = "wasm32"))]
async fn local_store<T: Send + 'static>(
    operation: impl FnOnce(acheflow_client::package::Store) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        let store = acheflow_client::package::Store::open(
            acheflow_client::package::client_root()?,
            acheflow_extension_api::parts::ExecutionHost::Client,
        )?;
        operation(store)
    })
    .await
    .map_err(|_| "Client package storage is unavailable.".to_owned())?
}
