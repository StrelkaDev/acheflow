use acheflow_media_api::{
    access::{Permission as P, Session},
    control::{ControlAction, ControlSnapshot},
    live::LiveStatus,
};

#[derive(Clone)]
pub(crate) struct Principal(pub Session);
impl Principal {
    pub fn allows(&self, permission: P) -> bool {
        self.0.device.permissions.contains(&permission)
    }
    pub fn all(&self, permissions: &[P]) -> bool {
        permissions.iter().all(|p| self.allows(*p))
    }
    pub fn any(&self, permissions: &[P]) -> bool {
        permissions.iter().any(|p| self.allows(*p))
    }
    pub fn server_accounts(&self) -> bool {
        self.workspace_management() && self.all(&[P::AccessManage, P::ChatRead])
    }
    pub fn workspace_management(&self) -> bool {
        self.all(&[
            P::WorkspaceManage,
            P::ScenesManage,
            P::MediaManage,
            P::ExtensionsConfigure,
            P::ExtensionsRun,
        ])
    }
    pub fn command(&self, action: &ControlAction) -> bool {
        match action {
            ControlAction::SendChat { .. } => self.allows(P::ChatSend),
            ControlAction::SetScene { .. } => self.allows(P::ScenesManage),
            ControlAction::CreatePoll { .. } | ControlAction::ClosePoll { .. } => {
                self.allows(P::InteractiveManage)
            }
            ControlAction::SetCommand { .. }
            | ControlAction::SetInteraction { .. }
            | ControlAction::TriggerInteraction { .. } => {
                self.all(&[P::InteractiveManage, P::ScenesManage])
            }
            ControlAction::SetExtension { config, .. } => {
                self.allows(P::ExtensionsRun)
                    && (config.is_none() || self.allows(P::ExtensionsConfigure))
            }
            ControlAction::SetClientCapabilities { .. }
            | ControlAction::ForwardExtension { .. } => {
                self.all(&[P::ExtensionsConfigure, P::ExtensionsRun])
            }
            // A generic guest panel action can run arbitrary declared effects.
            ControlAction::ExtensionEvent { .. } => self.all(&[
                P::ExtensionsConfigure,
                P::ExtensionsRun,
                P::ChatSend,
                P::ScenesManage,
                P::InteractiveManage,
            ]),
        }
    }
    pub fn snapshot(&self, mut snapshot: ControlSnapshot) -> ControlSnapshot {
        if !self.allows(P::ExtensionsConfigure) {
            for provider in &mut snapshot.server_providers {
                provider.account_id = None;
            }
            snapshot
                .client_executors
                .retain(|executor| !executor.client_id.starts_with("server-"));
        }
        if !self.workspace_management() {
            snapshot.workspace_fingerprint = None;
        }
        if !self.allows(P::ChatRead) {
            snapshot.chat = Default::default();
        }
        if !self.any(&[P::StudioView, P::ScenesManage]) {
            snapshot.scenes.clear();
            snapshot.source_frames.clear();
            snapshot.source_types.clear();
        }
        if !self.any(&[P::InteractiveView, P::InteractiveManage]) {
            snapshot.polls.clear();
            snapshot.commands.clear();
            snapshot.donations.clear();
            snapshot.interactions.clear();
        }
        if !self.allows(P::ExtensionsConfigure) {
            snapshot.extension_settings.clear();
            snapshot.client_capabilities.clear();
            snapshot.panels.clear();
        }
        if !self.allows(P::ExtensionsRun) {
            snapshot.client_executors.clear();
        }
        if !self.any(&[P::ExtensionsConfigure, P::ExtensionsRun]) {
            snapshot.extensions.clear();
        }
        if !self.any(&[P::StudioView, P::InteractiveView]) {
            snapshot.audience = Default::default();
            snapshot.history.clear();
        }
        for extension in &mut snapshot.extensions {
            extension.error = None;
        }
        snapshot
    }
    pub fn live(&self, mut status: LiveStatus) -> LiveStatus {
        if !self.allows(P::MediaManage) {
            for source in &mut status.sources {
                source.error = None;
            }
            for format in &mut status.formats {
                format.error = None;
                for output in &mut format.outputs {
                    output.error = None;
                }
            }
        }
        status
    }
    pub fn executor_id(&self, requested: &str) -> String {
        let digest = super::registry::hash(requested);
        format!("studio-{}-{}", self.0.device.id, &digest[..24])
    }
}
