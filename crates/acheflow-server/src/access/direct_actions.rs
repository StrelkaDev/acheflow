//! A fresh app-level check immediately before a client writes on the platform.
use super::*;
use acheflow_media_api::direct_action::{AuthorizeDirectAction, DirectActionPermission};
pub(super) async fn authorize(
    State(access): State<Arc<Access>>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<AuthorizeDirectAction>,
) -> Result<Json<DirectActionPermission>, Error> {
    let _command = access.host.command_lock.lock().await;
    let actor = access.registry.current(&principal.0, now())?;
    if !actor.device.permissions.contains(&P::ChatSend) {
        return Err(Error(403, "This device cannot send messages."));
    }
    if request.operation != "chat.send"
        || !acheflow_extension_api::integration::valid_provider_id(&request.provider)
        || !acheflow_extension_api::integration::valid_id(&request.channel_id, 128)
        || request
            .stream_id
            .as_ref()
            .is_some_and(|id| !acheflow_extension_api::integration::valid_id(id, 128))
    {
        return Err(Error(422, "Unsupported direct action or channel."));
    }
    if access.host.runtime.shutdown.is_cancelled() || !access.host.owns(&request.workspace_id) {
        return Err(Error(409, "The server workspace changed."));
    }
    let snapshot = access.host.snapshot(0, 0);
    if !snapshot.channels.iter().any(|c| {
        c.extension_id == request.provider
            && c.id == request.channel_id
            && c.stream_id == request.stream_id
    }) {
        return Err(Error(
            409,
            "The selected shared channel is no longer connected.",
        ));
    }
    let current = now();
    Ok(Json(DirectActionPermission {
        server_id: actor.server_id,
        user_id: actor.user.id,
        device_id: actor.device.id,
        workspace_id: request.workspace_id,
        provider: request.provider,
        channel_id: request.channel_id,
        stream_id: request.stream_id,
        server_time_ms: current,
        valid_until_ms: current + 5000,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(workspace: &str) -> AuthorizeDirectAction {
        AuthorizeDirectAction {
            workspace_id: workspace.into(),
            provider: "alpha".into(),
            channel_id: "channel".into(),
            stream_id: None,
            operation: "chat.send".into(),
        }
    }
    #[tokio::test]
    async fn direct_permission_uses_current_device_grants_and_shared_target_not_server_send_rights()
    {
        let directory = super::super::tests::Directory::new();
        let (host, workspace) = crate::control::tests::host();
        let access = Access::open(directory.path(), host.clone()).unwrap();
        let owner = access
            .registry
            .authenticate(&directory.owner(), now())
            .unwrap();
        let invite = access
            .registry
            .invite(
                &owner,
                [P::StudioView, P::ChatRead, P::ChatSend].into(),
                now(),
            )
            .unwrap();
        let paired = access
            .registry
            .pair(&invite.code, "Sender", "fixture", now())
            .unwrap();
        let principal = Principal(paired.session);
        host.snapshot(0, 0);
        host.services
            .lock()
            .unwrap()
            .integrations
            .register_channel(
                "alpha",
                acheflow_extension_api::integration::Channel {
                    stream_id: None,
                    id: "channel".into(),
                    extension_id: "alpha".into(),
                    label: "Shared read-only source".into(),
                    capabilities: vec!["chat.read".into()],
                    constraints: Default::default(),
                },
            )
            .unwrap();
        let permission = authorize(
            State(access.clone()),
            Extension(principal.clone()),
            Json(request(&workspace.id)),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(permission.user_id, principal.0.user.id);
        assert_eq!(permission.device_id, principal.0.device.id);
        let mut stale = request(&workspace.id);
        stale.stream_id = Some("previous-broadcast".into());
        assert!(
            authorize(
                State(access.clone()),
                Extension(principal.clone()),
                Json(stale)
            )
            .await
            .is_err()
        );
        assert!(!principal.server_accounts());
        assert!(
            authorize(
                State(access.clone()),
                Extension(principal.clone()),
                Json(request("foreign"))
            )
            .await
            .is_err()
        );
        let mut viewer = principal.clone();
        viewer.0.device.permissions.remove(&P::ChatSend);
        // Caller snapshots cannot grant or remove authority: registry remains authoritative.
        assert!(
            authorize(
                State(access.clone()),
                Extension(viewer),
                Json(request(&workspace.id))
            )
            .await
            .is_ok()
        );
        access
            .registry
            .revoke(&owner, &principal.0.device.id, now())
            .unwrap();
        assert!(
            authorize(
                State(access),
                Extension(principal),
                Json(request(&workspace.id))
            )
            .await
            .is_err()
        );
    }
}
