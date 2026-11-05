use super::*;
use acheflow_client::server::{LaunchConfig, LocalServer, Status};
use std::{path::PathBuf, sync::Arc};

/// Its runtime outlives the Dioxus tree, so close policy can finish after the
/// window closes instead of being canceled with component futures.
pub(crate) struct Service {
    runtime: tokio::runtime::Runtime,
    pub manager: Arc<LocalServer>,
}
impl Service {
    pub fn new() -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "Cannot start local server manager")?;
        let manager = {
            let _entered = runtime.enter();
            Arc::new(LocalServer::new().map_err(|e| e.to_string())?)
        };
        Ok(Self { runtime, manager })
    }
    pub fn finish(self) -> Result<(), String> {
        self.runtime
            .block_on(self.manager.shutdown())
            .map_err(|e| e.to_string())?;
        self.runtime
            .shutdown_timeout(std::time::Duration::from_secs(2));
        Ok(())
    }
}
fn directory() -> Result<PathBuf, String> {
    let path = crate::workspaces::storage_path()?;
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    Ok(path
        .parent()
        .ok_or("No local configuration directory")?
        .join("server"))
}
fn config(preferences: &ServerPreferences) -> Result<LaunchConfig, String> {
    preferences.validate()?;
    let mut config = LaunchConfig::bundled(directory()?).map_err(|e| e.to_string())?;
    config.http_port = preferences.http_port;
    config.rtmp_port = preferences.rtmp_port;
    Ok(config)
}
pub(super) fn provide(
    mut state: ServerState,
    project: Signal<Workspace>,
    catalog: Signal<crate::workspaces::Catalog>,
) {
    let manager = use_hook(try_consume_context::<Arc<LocalServer>>);
    use_context_provider(|| manager.clone());
    let preferences = use_memo(move || catalog.read().server_preferences());
    let policy_manager = manager.clone();
    use_effect(move || {
        if let Some(manager) = &policy_manager {
            manager.set_keep_running_on_exit(preferences().keep_running_on_exit);
        }
    });
    let watch_manager = manager.clone();
    use_future(move || {
        let manager = watch_manager.clone();
        async move {
            let Some(manager) = manager else {
                state
                    .error
                    .set("Local server manager is unavailable".into());
                return;
            };
            let mut receiver = manager.subscribe();
            loop {
                let snapshot = receiver.borrow_and_update().clone();
                if *state.process.peek() != snapshot {
                    state.process.set(snapshot);
                }
                let logs = manager.diagnostics();
                if *state.logs.peek() != logs {
                    state.logs.set(logs);
                }
                tokio::select! {
                    result = receiver.changed() => { if result.is_err() { break; } },
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                }
            }
        }
    });
    use_future(move || {
        let manager = manager.clone();
        async move {
            if preferences.peek().start_with_client
                && let Some(manager) = manager
            {
                launch(state, &manager, preferences.peek().clone(), project, false).await;
            }
        }
    });
    // Discovery of a configured local endpoint never grants ownership.
    use_resource(move || {
        let port = preferences().http_port;
        async move {
            loop {
                let info = acheflow_client::server::inspect_local(port)
                    .await
                    .ok()
                    .flatten();
                if *state.external.peek() != info {
                    state.external.set(info);
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    });
}
pub(super) fn action(
    mut state: ServerState,
    project: Signal<Workspace>,
    preferences: ServerPreferences,
    action: &'static str,
    manager: Option<Arc<LocalServer>>,
) {
    if *state.busy.peek() && action != "stop" {
        return;
    }
    let Some(manager) = manager else {
        state
            .error
            .set("Local server manager is unavailable".into());
        return;
    };
    *state.operation.write() += 1;
    let operation = *state.operation.peek();
    state.busy.set(true);
    spawn(async move {
        if action == "stop" {
            state.error.set(String::new());
            if let Err(error) = manager.stop().await
                && *state.operation.peek() == operation
            {
                state.error.set(error.to_string());
            }
        } else {
            launch(state, &manager, preferences, project, action == "restart").await;
        }
        if *state.operation.peek() == operation {
            state.busy.set(false);
        }
    });
}
async fn launch(
    mut state: ServerState,
    manager: &LocalServer,
    preferences: ServerPreferences,
    mut project: Signal<Workspace>,
    restart: bool,
) {
    state.busy.set(true);
    state.error.set(String::new());
    let operation = *state.operation.peek();
    // Never overwrite a remote target or a workspace switched during startup.
    let identity = (project.peek().id.clone(), project.peek().server_url.clone());
    let result = match config(&preferences) {
        Ok(config) => if restart {
            manager.restart(config).await
        } else {
            manager.start(config).await
        }
        .map_err(|e| e.to_string()),
        Err(error) => Err(error),
    };
    if *state.operation.peek() != operation {
        return;
    }
    match result {
        Ok(Status {
            endpoint: Some(endpoint),
            ..
        }) => {
            if identity == (project.peek().id.clone(), project.peek().server_url.clone())
                && local_target(&project.peek(), preferences.http_port)
            {
                project.write().server_url = endpoint;
            }
        }
        Ok(_) => {}
        Err(error) => state.error.set(error),
    }
    state.busy.set(false);
}
