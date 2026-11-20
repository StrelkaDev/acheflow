use super::*;
use axum::http::Request;
use std::io::{Cursor, Write};
use tower::ServiceExt;
fn archive(id: &str) -> Vec<u8> {
    let manifest = serde_json::json!({"id":id,"name":"Package fixture","api_version":2,"parts_version":1,"parts":{"server":{"kind":"logic","module":"guest.wasm","capabilities":["panel","status","storage"]}}});
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
        (
            "guest.wasm",
            include_bytes!("../../../../../extensions/sample/sample.wasm").to_vec(),
        ),
    ] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}
fn actor(
    access: &Access,
    owner: &acheflow_media_api::access::Session,
    permissions: &[P],
) -> String {
    let invitation = access
        .registry
        .invite(owner, permissions.iter().copied().collect(), now())
        .unwrap();
    access
        .registry
        .pair(&invitation.code, "Fixture device", "fixture", now())
        .unwrap()
        .token
        .unwrap()
}
async fn request(app: &Router, path: &str, token: Option<&str>, bytes: Vec<u8>) -> Response {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "studio.example")
        .header("x-acheflow-request", "1")
        .header(
            "content-type",
            if path.ends_with("stage") {
                "application/zip"
            } else {
                "application/json"
            },
        );
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    app.clone()
        .oneshot(request.body(Body::from(bytes)).unwrap())
        .await
        .unwrap()
}
#[tokio::test]
async fn upload_install_and_execution_consent_are_distinct_permissions_and_survive_reload() {
    let directory = crate::access::tests::Directory::new();
    let (host, _) = crate::control::tests::host();
    let access = Access::open(directory.path(), host.clone()).unwrap();
    let owner = access
        .registry
        .authenticate(&directory.owner(), now())
        .unwrap();
    let uploader = actor(&access, &owner, &[P::ExtensionsUpload]);
    let installer = actor(&access, &owner, &[P::ExtensionsInstall]);
    let runner = actor(&access, &owner, &[P::ExtensionsRun]);
    let app = access.protect(access.routes());
    let package = archive("package-fixture");
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/stage",
            None,
            package.clone()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/stage",
            Some(&installer),
            package.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let response = request(
        &app,
        "/api/extensions/packages/stage",
        Some(&uploader),
        package,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let review: PackageReview =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let command = serde_json::to_vec(&InstallPackage {
        digest: review.digest.clone(),
        expected_current: None,
    })
    .unwrap();
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/install",
            Some(&uploader),
            command.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/install",
            Some(&installer),
            command.clone()
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(host.extension(&review.id).is_some());
    assert!(!host.package_execution_allowed(&review.id));
    assert!(!host.guests.lock().await.contains_key(&review.id));
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/allow",
            Some(&installer),
            command.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/allow",
            Some(&runner),
            command
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(host.package_execution_allowed(&review.id));
    drop(app);
    drop(access);
    drop(host);
    let (host, _) = crate::control::tests::host();
    let restored = Access::open(directory.path(), host.clone()).unwrap();
    assert!(host.extension(&review.id).is_some());
    assert!(host.package_execution_allowed(&review.id));
    assert_eq!(restored.package_store.installed().unwrap().len(), 1);
    host.runtime.shutdown.cancel();
}
#[tokio::test]
async fn installing_a_replacement_retires_the_live_old_instance_and_requires_new_execution_consent()
{
    let directory = crate::access::tests::Directory::new();
    let (host, workspace) = crate::control::tests::host();
    let access = Access::open(directory.path(), host.clone()).unwrap();
    crate::extension_host::set_enabled(host.clone(), &workspace.id, "sample", true)
        .await
        .unwrap();
    assert!(host.guests.lock().await.contains_key("sample"));
    let app = access.protect(access.routes());
    let response = request(
        &app,
        "/api/extensions/packages/stage",
        Some(&directory.owner()),
        archive("sample"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let review: PackageReview =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let command = serde_json::to_vec(&InstallPackage {
        digest: review.digest,
        expected_current: None,
    })
    .unwrap();
    assert_eq!(
        request(
            &app,
            "/api/extensions/packages/install",
            Some(&directory.owner()),
            command
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert!(!host.guests.lock().await.contains_key("sample"));
    assert!(
        crate::extension_host::set_enabled(host.clone(), &workspace.id, "sample", true)
            .await
            .is_err()
    );
    host.runtime.shutdown.cancel();
}

#[tokio::test]
async fn slow_uploads_are_bounded_before_buffering_and_revocation_prevents_staging() {
    let directory = crate::access::tests::Directory::new();
    let (host, _) = crate::control::tests::host();
    let access = Access::open(directory.path(), host.clone()).unwrap();
    let owner = access
        .registry
        .authenticate(&directory.owner(), now())
        .unwrap();
    let token = actor(&access, &owner, &[P::ExtensionsUpload]);
    let device = access.registry.authenticate(&token, now()).unwrap();
    let app = access.protect(access.routes());
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    let bytes = archive("slow-fixture");
    let body = Body::from_stream(futures_util::stream::once(async move {
        let _ = entered.send(());
        let _ = wait.await;
        Ok::<_, std::io::Error>(bytes)
    }));
    let request = Request::builder()
        .method("POST")
        .uri("/api/extensions/packages/stage")
        .header("host", "studio.example")
        .header("x-acheflow-request", "1")
        .header("authorization", format!("Bearer {token}"))
        .body(body)
        .unwrap();
    let serving = app.clone();
    let pending = tokio::spawn(async move { serving.oneshot(request).await.unwrap() });
    ready.await.unwrap();
    assert_eq!(
        super::tests::request(
            &app,
            "/api/extensions/packages/stage",
            Some(&token),
            b"invalid".to_vec()
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    access
        .registry
        .revoke(&owner, &device.device.id, now())
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(pending.await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert!(access.package_store.installed().unwrap().is_empty());
    host.runtime.shutdown.cancel();
}
