//! Package upload, installation and execution consent are independent authorities.
use super::*;
use acheflow_extension_api::package::{InstallPackage, PackageReview};
use axum::body::Bytes;
fn current(access: &Access, principal: &Principal, permission: P) -> Result<(), Error> {
    let actor = access.registry.current(&principal.0, now())?;
    if !actor.device.permissions.contains(&permission) {
        return Err(Error(
            403,
            "This device does not have the required package permission.",
        ));
    }
    Ok(())
}
pub(super) async fn list(
    State(access): State<Arc<Access>>,
) -> Result<Json<Vec<PackageReview>>, Error> {
    let _permit = access
        .package_checks
        .try_acquire()
        .map_err(|_| Error(429, "Another package is being inspected."))?;
    let store = access.package_store.clone();
    let values = tokio::task::spawn_blocking(move || store.installed())
        .await
        .map_err(|_| Error(503, "Package inspection is unavailable."))?
        .map_err(|_| Error(503, "Installed package storage is unavailable."))?;
    Ok(Json(values.iter().map(|p| p.review().clone()).collect()))
}
pub(super) async fn stage(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Extension(_permit): Extension<Arc<tokio::sync::OwnedSemaphorePermit>>,
    bytes: Bytes,
) -> Result<Json<PackageReview>, Error> {
    current(&access, &principal, P::ExtensionsUpload)?;
    let actor = principal.clone();
    let target = access.clone();
    let bytes = bytes.to_vec();
    let result = tokio::task::spawn_blocking(move || {
        target.package_store.stage_checked(&bytes, now(), || {
            current(&target, &actor, P::ExtensionsUpload)
                .map_err(|_| "Package upload permission changed.".into())
        })
    })
    .await
    .map_err(|_| Error(503, "Package inspection is unavailable."))?;
    current(&access, &principal, P::ExtensionsUpload)?;
    result.map(Json).map_err(|_|Error(422,"Invalid or unsafe extension package; check its manifest, files, version and selected host part."))
}
pub(super) async fn install(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<InstallPackage>,
) -> Result<Json<PackageReview>, Error> {
    current(&access, &principal, P::ExtensionsInstall)?;
    let _permit = access
        .package_checks
        .try_acquire()
        .map_err(|_| Error(429, "Another package is being inspected."))?;
    let store = access.package_store.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        store.prepare(&request.digest, request.expected_current.as_deref())
    })
    .await
    .map_err(|_| Error(503, "Package inspection is unavailable."))?
    .map_err(|_| {
        Error(
            409,
            "Package expired or changed; upload and review it again.",
        )
    })?;
    let _command = access.host.command_lock.lock().await;
    current(&access, &principal, P::ExtensionsInstall)?;
    let id = prepared.review().id.clone();
    let available = match prepared.review().kind {
        acheflow_extension_api::parts::PartKind::Logic => {
            let installed = access.host.installed.read().unwrap();
            installed.contains_key(&id) || installed.len() < 8
        }
        acheflow_extension_api::parts::PartKind::Provider => {
            let catalog = access.host.provider_catalog.read().unwrap();
            catalog.installed.contains_key(&id) || catalog.installed.len() < 8
        }
    };
    if !available {
        return Err(Error(422, "The selected host catalog is full."));
    }
    access.package_store.commit(&prepared).map_err(|_| {
        Error(
            503,
            "Package could not be saved; the previous installation remains selected.",
        )
    })?;
    access.host.providers.stop_provider(&access.host, &id).await;
    if access.host.extension(&id).is_some() {
        let owner = access
            .host
            .runtime
            .workspace
            .read()
            .unwrap()
            .as_ref()
            .map(|w| w.id.clone());
        if let Some(owner) = owner {
            crate::extension_host::set_enabled(access.host.clone(), &owner, &id, false)
                .await
                .map_err(|_| {
                    Error(
                        503,
                        "Package saved, but the previous instance could not be retired.",
                    )
                })?;
        }
    }
    let review = prepared.review().clone();
    access.host.replace_package(prepared);
    crate::desktop::log(format_args!(
        "Extension package installed: extension={} digest={} device={}",
        id, review.digest, principal.0.device.id
    ));
    Ok(Json(review))
}
pub(super) async fn allow(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<InstallPackage>,
) -> Result<Json<serde_json::Value>, Error> {
    let _command = access.host.command_lock.lock().await;
    current(&access, &principal, P::ExtensionsRun)?;
    let id = access
        .package_store
        .id_for_digest(&request.digest)
        .ok_or(Error(409, "Installed package changed; review it again."))?;
    access
        .package_store
        .allow_execution(&id, &request.digest)
        .map_err(|_| Error(409, "Installed package changed; review it again."))?;
    access.host.providers.reconcile(&access.host).await;
    Ok(Json(serde_json::json!({"allowed":true})))
}

#[cfg(test)]
mod tests;
