//! Route domain commands by capability; no external service knowledge belongs here.
use crate::control::{ControlHost, Services};
use acheflow_extension_api::integration::*;
use acheflow_media_api::control::{ControlAction, ControlRequest};
use serde_json::{Value, json};

pub fn topic(effect: &Value) -> &str {
    match effect["type"].as_str() {
        Some("chat") => "chat.message",
        Some("donation") => "donation.received",
        Some("interaction") if effect["action"] == "register" => "interaction.registered",
        Some("interaction") => "interaction.triggered",
        Some("poll") => "poll.updated",
        Some("audience") => "audience.updated",
        Some("delete") => "chat.deleted",
        Some("delivery") => "chat.delivery",
        Some("channel") => "channel.updated",
        _ => "state.updated",
    }
}
fn dispatch(
    host: &ControlHost,
    request: &ControlRequest,
    channel: &Channel,
    name: &str,
    mut payload: Value,
) -> Result<(), String> {
    if !host.installed.contains_key(&channel.extension_id) {
        if name != "chat.send" {
            return Err("unavailable: client action is not supported".into());
        }
        return crate::client_bridge::enqueue_chat(
            host,
            request,
            channel,
            payload["text"].as_str().ok_or("missing text")?,
        );
    }
    if !host
        .services
        .lock()
        .unwrap()
        .instances
        .contains_key(&channel.extension_id)
    {
        return Err("unavailable: extension instance is not active".into());
    }
    payload["type"] = json!("command");
    payload["name"] = json!(name);
    payload["target"] = json!(channel.extension_id);
    payload["workspace_id"] = json!(request.workspace_id);
    payload["origin"] = json!("ui");
    payload["request_id"] = json!(request.request_id);
    payload["channel_id"] = json!(channel.id);
    host.events
        .send(payload)
        .map_err(|_| "unavailable: extension command receiver is closed".to_owned())?;
    Ok(())
}
pub fn command(host: &ControlHost, request: &ControlRequest) -> Result<(), String> {
    match &request.action {
        ControlAction::SendChat { targets, text } => {
            let channels = host
                .services
                .lock()
                .unwrap()
                .integrations
                .targets(targets, "chat.send")?;
            if channels.iter().any(|c| {
                c.constraints
                    .get("chat.max_chars")
                    .is_some_and(|limit| text.chars().count() as u64 > *limit)
            }) {
                return Err("invalid_argument: message exceeds a selected channel's limit".into());
            }
            host.services.lock().unwrap().interaction.begin_send(
                &request.request_id,
                text,
                &channels.iter().map(Channel::key).collect::<Vec<_>>(),
                crate::control::now_ms(),
            )?;
            for channel in channels {
                if let Err(error) =
                    dispatch(host, request, &channel, "chat.send", json!({"text":text}))
                {
                    host.services.lock().unwrap().interaction.delivery(
                        &channel.extension_id,
                        &channel.id,
                        &request.request_id,
                        Some(false),
                        None,
                        Some(error),
                    )?;
                }
            }
        }
        ControlAction::CreatePoll { poll } => {
            let channels = host
                .services
                .lock()
                .unwrap()
                .integrations
                .targets(&poll.channels, "poll.create")?;
            for channel in &channels {
                let limits = &channel.constraints;
                if limits
                    .get("poll.max_options")
                    .is_some_and(|n| poll.options.len() as u64 > *n)
                    || limits
                        .get("poll.min_duration_seconds")
                        .is_some_and(|n| u64::from(poll.duration_seconds) < *n)
                    || limits
                        .get("poll.max_duration_seconds")
                        .is_some_and(|n| u64::from(poll.duration_seconds) > *n)
                    || limits
                        .get("poll.max_question_chars")
                        .is_some_and(|n| poll.question.chars().count() as u64 > *n)
                    || limits
                        .get("poll.max_option_chars")
                        .is_some_and(|n| poll.options.iter().any(|s| s.chars().count() as u64 > *n))
                {
                    return Err(
                        "invalid_argument: poll exceeds a selected channel's declared limits"
                            .into(),
                    );
                }
            }
            let mut poll = poll.clone();
            poll.channels = channels.iter().map(Channel::key).collect();
            let id = host
                .services
                .lock()
                .unwrap()
                .interaction
                .create_poll(poll.clone(), crate::control::now_ms())?;
            for channel in channels {
                if let Err(error) = dispatch(
                    host,
                    request,
                    &channel,
                    "poll.create",
                    json!({"poll_id":id,"question":poll.question,"options":poll.options,"duration_seconds":poll.duration_seconds}),
                ) {
                    host.services.lock().unwrap().interaction.update_poll(
                        &channel.extension_id,
                        PollUpdate {
                            poll_id: id,
                            channel_id: channel.id,
                            external_id: None,
                            state: "failed".into(),
                            votes: vec![0; poll.options.len()],
                            error: Some(error),
                        },
                        crate::control::now_ms(),
                    )?;
                }
            }
        }
        ControlAction::ClosePoll { id, cancel } => {
            let poll = {
                let mut state = host.services.lock().unwrap();
                state.interaction.close_poll(*id, *cancel)?;
                state
                    .interaction
                    .polls(crate::control::now_ms())
                    .into_iter()
                    .find(|p| p.id == *id)
                    .ok_or("poll not found")?
            };
            let channels = host.services.lock().unwrap().integrations.channels();
            for target in poll.targets.iter().filter(|t| t.state == "closing") {
                if let Some(channel) = channels.iter().find(|c| c.key() == target.channel) {
                    dispatch(
                        host,
                        request,
                        channel,
                        "poll.close",
                        json!({"poll_id":id,"cancel":cancel}),
                    )?;
                }
            }
        }
        ControlAction::SetInteraction { interaction } => {
            validate_action(host, interaction.action.as_ref())?;
            let mut value = interaction.clone();
            let (origin, id) = interaction
                .id
                .split_once(':')
                .unwrap_or(("acheflow", interaction.id.as_str()));
            if origin != "acheflow" {
                host.services
                    .lock()
                    .unwrap()
                    .integrations
                    .interaction_enabled(origin, id)?;
            }
            value.id = id.into();
            host.services
                .lock()
                .unwrap()
                .integrations
                .configure_interaction(origin, value)?;
        }
        ControlAction::TriggerInteraction { id } => {
            let (origin, local) = id.split_once(':').ok_or("invalid interaction id")?;
            {
                let state = host.services.lock().unwrap();
                if !state.integrations.interaction_enabled(origin, local)? {
                    return Err("unavailable: interaction is disabled".into());
                }
                if origin != "acheflow" && !state.instances.contains_key(origin) {
                    return Err("unavailable: interaction extension is not active".into());
                }
            }
            let action = host
                .services
                .lock()
                .unwrap()
                .integrations
                .interaction(origin, local)?;
            if let Some(action) = action {
                host.apply_rotation(&request.workspace_id, &action)?;
            }
            let _=host.events.send(json!({"type":"event","name":"interaction.triggered","origin":"ui","workspace_id":request.workspace_id,"event":{"type":"interaction","action":"trigger","id":id},"request_id":request.request_id}));
        }
        _ => return Err("unsupported integration command".into()),
    }
    Ok(())
}
pub fn validate_action(host: &ControlHost, action: Option<&SceneAction>) -> Result<(), String> {
    if let Some(action) = action {
        let workspace = host.runtime.workspace.read().unwrap();
        if action.rotation.iter().any(|v| !v.is_finite())
            || !workspace.as_ref().is_some_and(|w| {
                w.scenes
                    .iter()
                    .find(|s| s.id == action.scene_id)
                    .and_then(|s| s.spatial.as_ref())
                    .is_some_and(|s| s.objects.iter().any(|o| o.id == action.object_id))
            })
        {
            return Err("invalid interaction scene/object".into());
        }
    }
    Ok(())
}
pub fn publish(
    state: &mut Services,
    origin: &str,
    effect: &Value,
    now: u64,
) -> Result<(Option<SceneAction>, bool), String> {
    match effect["type"].as_str() {
        Some("channel") => state.integrations.register_channel(
            origin,
            serde_json::from_value(effect["channel"].clone())
                .map_err(|_| "invalid channel descriptor")?,
        )?,
        Some("poll") => state.interaction.update_poll(
            origin,
            serde_json::from_value(effect["update"].clone()).map_err(|_| "invalid poll update")?,
            now,
        )?,
        Some("donation") => {
            let changed = state.integrations.donation(
                origin,
                serde_json::from_value(effect["donation"].clone())
                    .map_err(|_| "invalid donation")?,
            )?;
            return Ok((None, changed));
        }
        Some("delivery") => state.interaction.delivery(
            origin,
            effect["channel_id"]
                .as_str()
                .ok_or("missing delivery channel")?,
            effect["request_id"]
                .as_str()
                .ok_or("missing delivery request")?,
            match effect.get("sent") {
                Some(Value::Bool(value)) => Some(*value),
                Some(Value::Null) => None,
                _ => return Err("invalid delivery outcome".into()),
            },
            effect["external_id"].as_str().map(str::to_owned),
            effect["error"].as_str().map(str::to_owned),
        )?,
        Some("interaction") if effect["action"] == "register" => {
            state.integrations.register_interaction(
                origin,
                serde_json::from_value(effect["interaction"].clone())
                    .map_err(|_| "invalid interaction definition")?,
            )?
        }
        Some("interaction") if effect["action"] == "trigger" => {
            let event = effect["event_id"]
                .as_str()
                .filter(|id| valid_id(id, 256))
                .ok_or("missing interaction event id")?;
            let key = (origin.to_owned(), event.to_owned());
            if state.interaction_events.contains(&key) {
                return Ok((None, false));
            }
            state.interaction_events.push_back(key);
            if state.interaction_events.len() > 2000 {
                state.interaction_events.pop_front();
            }
            if !state.integrations.interaction_enabled(
                origin,
                effect["id"].as_str().ok_or("missing interaction id")?,
            )? {
                return Ok((None, false));
            }
            return Ok((
                state.integrations.interaction(
                    origin,
                    effect["id"].as_str().ok_or("missing interaction id")?,
                )?,
                true,
            ));
        }
        _ => return Err("unsupported domain event".into()),
    }
    Ok((None, true))
}

pub fn validate_effect(host: &ControlHost, origin: &str, effect: &Value) -> Result<(), String> {
    let extension = host.installed.get(origin).ok_or("unknown extension")?;
    let kind = effect["type"].as_str().ok_or("missing effect type")?;
    if !extension.manifest.capabilities.iter().any(|c| c == kind) {
        return Err("forbidden: extension capability is not granted".into());
    }
    if kind == "channel"
        && let Some(capabilities) = effect["channel"]["capabilities"].as_array()
    {
        for capability in capabilities {
            let capability = capability.as_str().ok_or("invalid channel capability")?;
            let domain = capability.split('.').next().unwrap_or_default();
            if !extension
                .manifest
                .subscriptions
                .iter()
                .any(|s| s == capability)
                || !extension.manifest.capabilities.iter().any(|s| s == domain)
            {
                return Err("forbidden: channel capability is not granted".into());
            }
        }
    }
    if kind == "interaction"
        && effect["action"] == "register"
        && !effect["interaction"]["action"].is_null()
    {
        if !extension.manifest.capabilities.iter().any(|c| c == "scene") {
            return Err("forbidden: scene capability is not granted".into());
        }
        let action: SceneAction = serde_json::from_value(effect["interaction"]["action"].clone())
            .map_err(|_| "invalid interaction action")?;
        validate_action(host, Some(&action))?;
    }
    if kind == "panel"
        && let Some(widgets) = effect["panel"]["widgets"].as_array()
    {
        for widget in widgets {
            if widget["type"] == "link" {
                return Err(
                    "server logic extensions cannot publish provider authorization links".into(),
                );
            }
        }
    }
    Ok(())
}
