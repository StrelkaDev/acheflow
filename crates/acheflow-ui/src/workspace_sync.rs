//! One revision-ordered workspace feed shared by the editor, Studio and chat.
use crate::{control_client::ControlClient, project::Workspace, workspaces::Catalog};
use dioxus_native::prelude::*;

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceSync {
    pub following: Signal<bool>,
    pub ready: Signal<bool>,
    pub error: Signal<Option<String>>,
    project: Signal<Workspace>,
    catalog: Signal<Catalog>,
    control: ControlClient,
}
impl WorkspaceSync {
    fn receive(
        mut self,
        state: &acheflow_media_api::server_workspace::WorkspaceState,
    ) -> Result<(), String> {
        let mut next = self.catalog.peek().clone();
        let update = next.sync_server(&self.project.peek(), state)?;
        if let Some(update) = update {
            if *self.project.peek() != update.project {
                self.project.set(update.project);
            }
            if *self.catalog.peek() != next {
                self.catalog.set(next);
            }
            self.control.sync_conflict.set(update.conflict);
            self.ready.set(true);
        } else if state.summary.is_none() {
            self.ready.set(false);
        }
        Ok(())
    }
    pub fn local(mut self) {
        self.following.set(false);
        self.control.sync_conflict.set(false);
    }
    pub fn resolve(mut self, use_server: bool) {
        let mut next = self.catalog.peek().clone();
        let local = self.project.peek().clone();
        if let Some(update) = next.resolve_server(&local, use_server) {
            self.project.set(update.project);
            self.catalog.set(next);
            self.control.sync_conflict.set(false);
            self.error.set(None);
        }
    }
    pub fn applied(mut self, sent: &Workspace, revision: u64) {
        if self.project.peek().id != sent.id || self.project.peek().server_url != sent.server_url {
            return;
        }
        let Some(session) = try_consume_context::<crate::server_access::AccessState>()
            .and_then(|s| s.session.peek().clone())
        else {
            return;
        };
        let mut next = self.catalog.peek().clone();
        next.update_current(self.project.peek().clone());
        if next.applied_server(sent, revision, &session.server_id) {
            self.project.write().edit_revision = revision;
            next.update_current(self.project.peek().clone());
            self.catalog.set(next);
            self.control.sync_conflict.set(false);
            self.following.set(true);
        }
    }
}
pub(crate) fn provide(
    project: Signal<Workspace>,
    catalog: Signal<Catalog>,
    control: ControlClient,
) -> WorkspaceSync {
    let mut sync = WorkspaceSync {
        project,
        catalog,
        control,
        following: use_signal(|| true),
        ready: use_signal(|| false),
        error: use_signal(|| None),
    };
    use_context_provider(|| sync);
    let access = use_context::<crate::server_access::AccessState>();
    let identity = use_memo(move || {
        (
            crate::media::server_url(&project.read(), "/api/workspace"),
            access.session.read().clone(),
            (sync.following)(),
        )
    });
    use_resource(move || {
        let owner = identity();
        sync.ready.set(false);
        sync.error.set(None);
        if owner.1.is_none() || !owner.2 {
            sync.control.sync_conflict.set(false);
        }
        async move {
            let (Ok(_), Some(session), true) = &owner else {
                return;
            };
            let mut failures = 0u8;
            loop {
                let requested = project.peek().clone();
                let result = crate::server_workspace::fetch(&requested).await;
                if identity() != owner {
                    return;
                }
                match result {
                    Ok(state) if state.server_id == session.server_id => {
                        match sync.receive(&state) {
                            Ok(()) => {
                                sync.error.set(None);
                                failures = 0;
                            }
                            Err(error) => sync.error.set(Some(error)),
                        }
                    }
                    Ok(_) => sync.error.set(Some(
                        "The connected server identity changed. Reconnect this device.".into(),
                    )),
                    Err(error) => {
                        failures = failures.saturating_add(1);
                        sync.error.set(Some(error));
                    }
                }
                crate::scene_preview::pause(crate::live_client::polling_delay(
                    true, failures, 1000,
                ))
                .await;
            }
        }
    });
    sync
}
#[component]
pub(crate) fn Notice() -> Element {
    let mut sync = use_context::<WorkspaceSync>();
    let conflict = (sync.control.sync_conflict)();
    let busy = (sync.control.busy)()
        || try_consume_context::<crate::live_client::LiveClient>()
            .is_some_and(|live| (live.busy)());
    rsx! {if conflict {
        div {class:"panel-note workspace-sync-note",role:"alert",span {"This workspace changed on the server. Your local draft is preserved."}
            button {disabled:busy,onclick:move |_|sync.resolve(true),"Use server workspace"}
            button {disabled:busy,onclick:move |_|sync.resolve(false),"Keep local edits"}
        }
    }else if let Some(error)=(sync.error)(){div {class:"panel-note workspace-sync-note",role:"status","Workspace sync: {error}"}}
    else if !(sync.following)(){div {class:"panel-note workspace-sync-note",span {"Editing a local draft. The server workspace continues running."}button {disabled:busy,onclick:move |_|sync.following.set(true),"Follow server workspace"}}}}
}

#[cfg(test)]
mod tests;
