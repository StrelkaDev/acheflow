//! Real server WASM instances exercise forwarding through the control boundary.
use crate::control::{ControlHost, now_ms};
use acheflow_extension_api::{PanelWidget, integration::GuestManifest};
use acheflow_media_api::control::{ControlAction, ControlRequest, ControlSnapshot};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

fn host() -> Arc<ControlHost> {
    let (mut host, _) = crate::control::tests::host();
    let inner = Arc::get_mut(&mut host).unwrap();
    inner.installed.clear();
    for id in ["alpha", "beta"] {
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../../extensions/sample/manifest.json")).unwrap();
        manifest["id"] = id.into();
        manifest["name"] = id.into();
        manifest["forwarding"] = json!([{ "id": "increment", "mode": "event" }]);
        let manifest: GuestManifest = serde_json::from_value(manifest).unwrap();
        manifest.validate().unwrap();
        inner.installed.insert(
            id.into(),
            crate::extension_registry::Installed {
                manifest,
                module: include_bytes!("../../../extensions/sample/sample.wasm").to_vec(),
            },
        );
    }
    *inner.services.lock().unwrap() = Default::default();
    host.snapshot(0, 200);
    host
}

fn request(host: &ControlHost, action: ControlAction) -> ControlRequest {
    let state = host.snapshot(0, 200);
    ControlRequest {
        workspace_id: state.workspace_id.unwrap(),
        request_id: format!(
            "{}:{}",
            now_ms(),
            acheflow_media_api::workspace::new_token()
        ),
        expected_revision: state.revision,
        action,
    }
}

async fn set_enabled(host: &Arc<ControlHost>, id: &str, enabled: bool) {
    host.execute(request(
        host,
        ControlAction::SetExtension {
            id: id.into(),
            enabled,
            config: None,
        },
    ))
    .await
    .unwrap();
}

fn count(state: &ControlSnapshot, id: &str) -> Option<u64> {
    state
        .panels
        .iter()
        .find(|panel| panel.extension_id == id)
        .and_then(|panel| {
            panel.widgets.iter().find_map(|widget| match widget {
                PanelWidget::Metric { label, value } if label == "Actions" => value.parse().ok(),
                _ => None,
            })
        })
}

async fn wait_count(host: &ControlHost, id: &str, expected: u64) -> ControlSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = host.snapshot(0, 200);
            if count(&state, id) == Some(expected) {
                return state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the real sample guest did not publish its expected action count")
}

fn instance(host: &ControlHost, id: &str) -> String {
    host.snapshot(0, 200)
        .extensions
        .into_iter()
        .find(|status| status.id == id)
        .and_then(|status| status.instance_id)
        .expect("a live server extension must expose its current instance")
}

fn forward(url: &str, instance_id: &str) -> ControlAction {
    ControlAction::ForwardExtension {
        url: url.into(),
        instance_id: instance_id.into(),
    }
}

async fn enable_both(host: &Arc<ControlHost>) {
    for id in ["alpha", "beta"] {
        set_enabled(host, id, true).await;
        wait_count(host, id, 0).await;
    }
}

#[tokio::test]
async fn forwarded_actions_reach_only_the_target_real_guest_and_retries_do_not_repeat_them() {
    let host = host();
    enable_both(&host).await;
    let intent = request(
        &host,
        forward(
            "https://app.acheflow.com/server/fwd/alpha/increment",
            &instance(&host, "alpha"),
        ),
    );

    let first = host.execute(intent.clone()).await.unwrap();
    let state = wait_count(&host, "alpha", 1).await;
    assert_eq!(count(&state, "beta"), Some(0));

    let retry = host.execute(intent).await.unwrap();
    assert_eq!(
        retry.revision, first.revision,
        "retrying the same intent must not apply it or advance control revision"
    );
    assert_eq!(count(&retry, "alpha"), Some(1));
    assert_eq!(count(&retry, "beta"), Some(0));
    host.runtime.shutdown.cancel();
}

#[tokio::test]
async fn server_forwarding_rejects_wrong_recipient_missing_target_undeclared_action_and_workspace()
{
    let host = host();
    enable_both(&host).await;
    let current = instance(&host, "alpha");
    for url in [
        "https://app.acheflow.com/fwd/alpha/increment",
        "https://app.acheflow.com/server/fwd/missing/increment",
        "https://app.acheflow.com/server/fwd/alpha/undeclared",
    ] {
        assert!(
            host.execute(request(&host, forward(url, &current)))
                .await
                .is_err(),
            "accepted {url}"
        );
    }
    let mut other_workspace = request(
        &host,
        forward(
            "https://app.acheflow.com/server/fwd/alpha/increment",
            &current,
        ),
    );
    other_workspace.workspace_id = "another-workspace".into();
    assert!(host.execute(other_workspace).await.is_err());

    let state = host.snapshot(0, 200);
    assert_eq!(count(&state, "alpha"), Some(0));
    assert_eq!(count(&state, "beta"), Some(0));
    host.runtime.shutdown.cancel();
}

#[tokio::test]
async fn stopped_and_restarted_instances_cannot_receive_an_old_forwarding_intent() {
    let host = host();
    enable_both(&host).await;
    let old_instance = instance(&host, "alpha");
    let url = "https://app.acheflow.com/server/fwd/alpha/increment";

    set_enabled(&host, "alpha", false).await;
    assert!(
        host.execute(request(&host, forward(url, &old_instance)))
            .await
            .is_err()
    );
    assert!(
        host.snapshot(0, 200)
            .panels
            .iter()
            .all(|panel| panel.extension_id != "alpha")
    );

    set_enabled(&host, "alpha", true).await;
    wait_count(&host, "alpha", 0).await;
    let new_instance = instance(&host, "alpha");
    assert_ne!(old_instance, new_instance);
    assert!(
        host.execute(request(&host, forward(url, &old_instance)))
            .await
            .is_err()
    );

    host.execute(request(&host, forward(url, &new_instance)))
        .await
        .unwrap();
    let state = wait_count(&host, "alpha", 1).await;
    assert_eq!(count(&state, "beta"), Some(0));
    host.runtime.shutdown.cancel();
}

#[test]
fn server_manifests_cannot_declare_credential_callback_delivery() {
    let mut manifest: Value =
        serde_json::from_str(include_str!("../../../extensions/sample/manifest.json")).unwrap();
    manifest["forwarding"] = json!([{ "id": "oauth", "mode": "callback" }]);
    let manifest: GuestManifest = serde_json::from_value(manifest).unwrap();
    assert!(manifest.validate().is_err());
}

#[tokio::test]
async fn native_activation_rechecks_expiry_after_waiting_for_the_control_lock() {
    use acheflow_launcher::ActivationError;
    use std::sync::atomic::{AtomicBool, Ordering};
    let host = host();
    enable_both(&host).await;
    let lock = host.command_lock.lock().await;
    let live = AtomicBool::new(true);
    let delivery = crate::native_activation::dispatch(
        &host,
        "https://app.acheflow.com/server/fwd/alpha/increment",
        || live.load(Ordering::Acquire),
    );
    tokio::pin!(delivery);
    tokio::select! {
        result = &mut delivery => panic!("delivery bypassed the command lock: {result:?}"),
        _ = tokio::task::yield_now() => {},
    }
    live.store(false, Ordering::Release);
    drop(lock);
    assert_eq!(delivery.await, Err(ActivationError::Expired));
    assert_eq!(count(&host.snapshot(0, 200), "alpha"), Some(0));
    host.runtime.shutdown.cancel();
}

#[tokio::test]
async fn native_activation_rechecks_expiry_at_the_guest_inbox() {
    use acheflow_launcher::ActivationError;
    use std::sync::atomic::{AtomicBool, Ordering};
    let host = host();
    enable_both(&host).await;
    let lock = host.guests.lock().await;
    let live = AtomicBool::new(true);
    let delivery = crate::native_activation::dispatch(
        &host,
        "https://app.acheflow.com/server/fwd/alpha/increment",
        || live.load(Ordering::Acquire),
    );
    tokio::pin!(delivery);
    tokio::select! {
        result = &mut delivery => panic!("delivery bypassed the guest lock: {result:?}"),
        _ = tokio::task::yield_now() => {},
    }
    live.store(false, Ordering::Release);
    drop(lock);
    assert_eq!(delivery.await, Err(ActivationError::Expired));
    assert_eq!(count(&host.snapshot(0, 200), "alpha"), Some(0));
    host.runtime.shutdown.cancel();
}

#[tokio::test]
async fn native_activation_targets_only_enabled_declared_server_guests() {
    use acheflow_launcher::ActivationError;
    let host = host();
    enable_both(&host).await;
    let alpha = "https://app.acheflow.com/server/fwd/alpha/increment";
    crate::native_activation::dispatch(&host, alpha, || true)
        .await
        .unwrap();
    let state = wait_count(&host, "alpha", 1).await;
    assert_eq!(count(&state, "beta"), Some(0));
    for (url, error) in [
        (
            "https://app.acheflow.com/fwd/alpha/increment",
            ActivationError::InvalidLink,
        ),
        (
            "https://app.acheflow.com/server/fwd/missing/increment",
            ActivationError::NotFound,
        ),
        (
            "https://app.acheflow.com/server/fwd/alpha/undeclared",
            ActivationError::Rejected,
        ),
    ] {
        assert_eq!(
            crate::native_activation::dispatch(&host, url, || true).await,
            Err(error)
        );
    }
    set_enabled(&host, "alpha", false).await;
    assert_eq!(
        crate::native_activation::dispatch(&host, alpha, || true).await,
        Err(ActivationError::Disabled)
    );
    assert_eq!(
        crate::native_activation::dispatch(&host, alpha, || false).await,
        Err(ActivationError::Expired)
    );
    *host.runtime.workspace.write().unwrap() = None;
    assert_eq!(
        crate::native_activation::dispatch(&host, alpha, || true).await,
        Err(ActivationError::NotReady)
    );
    host.runtime.shutdown.cancel();
}

#[tokio::test]
async fn native_activation_cannot_follow_a_workspace_switch_while_queued() {
    use acheflow_launcher::ActivationError;
    let host = host();
    enable_both(&host).await;
    let lock = host.command_lock.lock().await;
    let delivery = crate::native_activation::dispatch(
        &host,
        "https://app.acheflow.com/server/fwd/alpha/increment",
        || true,
    );
    tokio::pin!(delivery);
    tokio::select! {
        result = &mut delivery => panic!("delivery bypassed the command lock: {result:?}"),
        _ = tokio::task::yield_now() => {},
    }
    host.runtime.workspace.write().unwrap().as_mut().unwrap().id = "replacement-workspace".into();
    drop(lock);
    assert_eq!(delivery.await, Err(ActivationError::Rejected));
    assert!(host.snapshot(0, 200).panels.is_empty());
    host.runtime.shutdown.cancel();
}
