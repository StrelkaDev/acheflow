//! One revision-ordered workspace feed shared by the editor, Studio and chat.
use crate::{
    control_client::ControlClient,
    project::{Orientation, Workspace},
    workspaces::Catalog,
};
use acheflow_media_api::audio::{MixerAck, MixerEdit, MixerRequest};
use acheflow_media_api::{
    live::{
        OutputProfile, OutputProfileAck, OutputProfileRequest, OutputTarget, SceneContent,
        SceneEdit, SceneSettingsAck, SceneSettingsRequest, SourceEdit, SourceSettingsAck,
        SourceSettingsRequest, TakeSceneAck, TakeSceneRequest,
    },
    workspace::Source,
};
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

pub(crate) struct SceneSaveState {
    pub saved: bool,
    pub changed: bool,
    pub available: bool,
}

impl WorkspaceSync {
    pub(crate) fn output_profiles(&self, orientation: Orientation) -> Option<Vec<OutputProfile>> {
        if !(self.following)() || (self.control.sync_conflict)() {
            return None;
        }
        Some(
            self.catalog
                .read()
                .editable_base(&self.project.read())?
                .stream(orientation)
                .outputs
                .iter()
                .map(OutputProfile::of)
                .collect(),
        )
    }

    pub(crate) fn changed_output_profile(&self, target: OutputTarget) -> Option<OutputProfile> {
        let saved = self.output_profiles(target.orientation)?;
        let project = self.project.read();
        let draft = &project.stream(target.orientation).outputs;
        if saved.len() != draft.len() {
            return None;
        }
        let profile = OutputProfile::of(draft.get(target.index)?);
        (saved.get(target.index)? != &profile).then_some(profile)
    }

    pub(crate) fn applied_output_profile(
        mut self,
        request: &OutputProfileRequest,
        ack: OutputProfileAck,
    ) -> bool {
        let Some(session) = try_consume_context::<crate::server_access::AccessState>()
            .and_then(|state| state.session.peek().clone())
        else {
            return false;
        };
        let mut next = self.catalog.peek().clone();
        if !next.applied_delta(
            &self.project.peek(),
            request.expected_revision,
            ack.revision,
            &session.server_id,
            |base| request.apply(base),
        ) {
            return false;
        }
        self.project.write().edit_revision = ack.revision;
        next.update_current(self.project.peek().clone());
        self.catalog.set(next);
        true
    }

    pub(crate) fn scene_save_state(&self, id: u32) -> Option<SceneSaveState> {
        if !(self.following)() || (self.control.sync_conflict)() {
            return None;
        }
        let project = self.project.read();
        let draft = project.scenes.iter().find(|scene| scene.id == id)?;
        let catalog = self.catalog.read();
        let server = catalog.editable_base(&project)?;
        let saved = server.scenes.iter().find(|scene| scene.id == id);
        let base = &server.stream(draft.orientation).base;
        let has_source = |id: u32| server.sources.iter().any(|source| source.id == id);
        Some(SceneSaveState {
            saved: saved.is_some(),
            changed: saved.is_none_or(|saved| !SceneContent::unchanged(draft, saved, base)),
            available: *base == project.stream(draft.orientation).base
                && draft
                    .visual_layers()
                    .all(|layer| has_source(layer.source_id))
                && (saved.is_some()
                    || draft
                        .audio
                        .strips
                        .iter()
                        .all(|strip| has_source(strip.source_id))),
        })
    }

    pub(crate) fn applied_scene(
        mut self,
        request: &SceneSettingsRequest,
        ack: SceneSettingsAck,
    ) -> bool {
        let Some(session) = try_consume_context::<crate::server_access::AccessState>()
            .and_then(|state| state.session.peek().clone())
        else {
            return false;
        };
        let mut next = self.catalog.peek().clone();
        if !next.applied_delta(
            &self.project.peek(),
            request.expected_revision,
            ack.revision,
            &session.server_id,
            |base| request.edit.apply(base, request.scene_id),
        ) {
            return false;
        }
        {
            let mut project = self.project.write();
            if matches!(request.edit, SceneEdit::Remove) {
                let _ = request.edit.apply(&mut project, request.scene_id);
            }
            project.edit_revision = ack.revision;
        }
        next.update_current(self.project.peek().clone());
        self.catalog.set(next);
        true
    }

    pub(crate) fn can_edit_workspace(&self) -> bool {
        (self.following)()
            && !(self.control.sync_conflict)()
            && self
                .catalog
                .read()
                .editable_base(&self.project.read())
                .is_some()
    }

    pub(crate) fn saved_source(&self, id: u32) -> Option<Source> {
        if !(self.following)() || (self.control.sync_conflict)() {
            return None;
        }
        self.catalog
            .read()
            .editable_base(&self.project.read())?
            .sources
            .iter()
            .find(|source| source.id == id)
            .cloned()
    }

    pub(crate) fn applied_source(
        mut self,
        request: &SourceSettingsRequest,
        ack: SourceSettingsAck,
    ) -> bool {
        let Some(session) = try_consume_context::<crate::server_access::AccessState>()
            .and_then(|state| state.session.peek().clone())
        else {
            return false;
        };
        let mut next = self.catalog.peek().clone();
        if !next.applied_delta(
            &self.project.peek(),
            request.expected_revision,
            ack.revision,
            &session.server_id,
            |base| request.edit.apply(base, request.source_id).map(|_| ()),
        ) {
            return false;
        }
        {
            let mut project = self.project.write();
            if matches!(request.edit, SourceEdit::Remove) {
                project.remove_source(request.source_id);
            }
            project.edit_revision = ack.revision;
        }
        next.update_current(self.project.peek().clone());
        self.catalog.set(next);
        true
    }

    pub(crate) fn can_take_scene(&self, orientation: Orientation, scene: u32) -> bool {
        (self.following)()
            && !(self.control.sync_conflict)()
            && self
                .catalog
                .read()
                .can_take_scene(&self.project.read(), orientation, scene)
    }

    pub(crate) fn applied_program(mut self, request: &TakeSceneRequest, ack: TakeSceneAck) -> bool {
        let Some(session) = try_consume_context::<crate::server_access::AccessState>()
            .and_then(|state| state.session.peek().clone())
        else {
            return false;
        };
        let mut next = self.catalog.peek().clone();
        if !next.applied_delta(
            &self.project.peek(),
            request.expected_revision,
            ack.revision,
            &session.server_id,
            |base| request.apply(base),
        ) {
            return false;
        }
        let mut project = self.project.peek().clone();
        // A locally deleted target may still exist on the server. Keep the
        // deletion draft intact and let the applied program status report it.
        let _ = request.apply(&mut project);
        project.edit_revision = ack.revision;
        next.update_current(project.clone());
        self.project.set(project);
        self.catalog.set(next);
        true
    }
    pub(crate) fn mixer_has_target(&self, edit: &MixerEdit) -> bool {
        *self.following.peek()
            && !*self.control.sync_conflict.peek()
            && self
                .catalog
                .peek()
                .mixer_has_target(&self.project.peek(), edit)
    }

    pub(crate) fn applied_mixer(mut self, request: &MixerRequest, ack: MixerAck) -> bool {
        let Some(session) = try_consume_context::<crate::server_access::AccessState>()
            .and_then(|state| state.session.peek().clone())
        else {
            return false;
        };
        let mut next = self.catalog.peek().clone();
        if !next.applied_mixer(&self.project.peek(), request, ack, &session.server_id) {
            return false;
        }
        self.project.write().edit_revision = ack.revision;
        next.update_current(self.project.peek().clone());
        self.catalog.set(next);
        true
    }
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
    let live = use_context::<crate::live_client::LiveClient>();
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
                if *live.scoped_pending.peek() || *live.busy.peek() {
                    crate::scene_preview::pause(100).await;
                    continue;
                }
                let requested = project.peek().clone();
                let result = crate::server_workspace::fetch(&requested).await;
                if identity() != owner {
                    return;
                }
                if *live.scoped_pending.peek() || *live.busy.peek() {
                    continue;
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
