use crate::{
    live::Live,
    live_client::use_live_client,
    sparks::SparksWidget,
    workspaces::{WorkspaceSwitcher, Workspaces, use_workspaces},
};
use base64::Engine;
use dioxus_native::{CustomWidgetAttr, prelude::*};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn dashboard() -> Element {
    let logo = use_hook(|| {
        format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD
                .encode(include_bytes!("../assets/achedev-logo.svg"))
        )
    });
    let backend = use_signal(|| "Starting GPU...".to_owned());
    let mut page = use_signal(|| "Overview");
    let (project, mut catalog, storage_status) = use_workspaces();
    crate::server_settings::provide(project, catalog);
    let effects = use_signal(|| catalog.peek().background_animation());
    use_effect(move || {
        let enabled = effects();
        if catalog.peek().background_animation() != enabled {
            catalog.write().set_background_animation(enabled);
        }
    });
    let workspace_open = use_signal(|| false);
    let live = use_live_client(project);
    let control = crate::control_client::use_control_client(project);
    crate::diagnostics::observe(live, control);
    crate::interaction::use_chat_draft(project);
    #[cfg(not(target_arch = "wasm32"))]
    let activation_notice = crate::native_activation::provide(project, page);
    rsx! {
        style {{include_str!("dashboard.css")}}
        div {class:"app",
            aside {class:"sidebar",
                div {class:"brand",img {class:"brand-mark",alt:"Acheflow",src:logo}"acheflow"}
                div {class:"workspace-label","WORKSPACE"}
                div {class:"sidebar-workspaces",WorkspaceSwitcher {project,open:workspace_open}}
                nav {for item in ["Overview","Live","Extensions","Settings"] {
                    button {class:if page()==item{"nav-item selected"}else{"nav-item"},onclick:move |_|page.set(item),
                        span {class:"nav-symbol",if item=="Live"{"▷"}else if item=="Settings"{"⚙"}else{"▦"}}"{item}"
                    }
                }}
                div {class:"sidebar-footer",
                    div {class:"local-badge",span {class:"status-dot"}"Local workspace"}
                    p {class:"sidebar-backend","{backend}"}
                    p {class:"app-version","v{APP_VERSION}"}
                }
            }
            div {class:"workspace",
                {native_activation_notice(
                    #[cfg(not(target_arch = "wasm32"))]
                    activation_notice,
                )}
                if effects(){SparksBackground {backend}}
                Workspaces {project,catalog,storage_status,open:workspace_open}
                div {class:if live.running(){"workspace-body with-live-chat"}else{"workspace-body"},
                div {class:"tab-viewport",for tab in ["Overview","Live","Extensions","Settings"] {
                    div {key:"{tab}",class:if page()==tab{"tab-scroll active"}else{"tab-scroll"},"aria-hidden":if page()==tab{"false"}else{"true"},
                        if tab=="Live"{Live {key:"{catalog.read().active_id()}",project,visible:page()=="Live"}}else{
                            main {class:"content",
                                if tab=="Overview"{
                                    div {class:"page-heading",div {h1 {"Overview"}p {"Your broadcast workspace, in one place."}}span {class:"badge",if live.running(){"Running"}else{"Idle"}}}
                                    div {class:"stats",
                                        Stat {title:"EXPECTED INPUTS",value:project.read().sources.len().to_string(),detail:"RTMP sources"}
                                        Stat {title:"SCENES",value:project.read().scenes.len().to_string(),detail:"Horizontal + vertical"}
                                        Stat {title:"RESTREAM OUTPUTS",value:(project.read().horizontal.outputs.len()+project.read().vertical.outputs.len()).to_string(),detail:"Configured destinations"}
                                        Stat {title:"SESSION",value:if live.running(){"Running"}else{"Idle"},detail:"Media worker state"}
                                    }
                                    section {class:"panel",div {class:"panel-heading",h2 {"Prepare your broadcast"}}div {class:"padded",h3 {"Sources → Scenes → Studio"}p {"Register expected RTMP inputs in Live / Sources. Build compositions in Live / Scenes, set separate output profiles in Settings and monitor both formats in Studio."}p {"FFmpeg ingests authorized RTMP sources and renders scenes. Manage the live session in Studio."}}}
                                }

                                if tab=="Extensions"{
                                    crate::interaction::ExtensionsPage {}
                                }
                                if tab=="Settings"{GeneralSettings {effects,project,catalog,storage_status}}
                                footer {class:"content-footer",span {"ACHEFLOW / STREAM CONTROL"}}
                            }
                        }
                    }
                }}
                if live.running(){aside {class:"live-chat-sidebar","aria-label":"Live chat",crate::interaction::ChatPanel {}}}
                }
            }
        }
    }
}

fn native_activation_notice(#[cfg(not(target_arch = "wasm32"))] value: Signal<String>) -> Element {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rsx! {crate::native_activation::Notice{value}}
    }
    #[cfg(target_arch = "wasm32")]
    {
        VNode::empty()
    }
}
#[component]
pub(crate) fn GeneralSettings(
    mut effects: Signal<bool>,
    project: Signal<crate::project::Workspace>,
    catalog: Signal<crate::workspaces::Catalog>,
    storage_status: Signal<String>,
) -> Element {
    let mut section = use_signal(|| "General");
    rsx! {
        div {class:"page-heading",div {h1 {"Settings"}p {"Server connection, desktop behavior and appearance."}}}
        nav {class:"settings-tabs","aria-label":"Settings sections",
            crate::diagnostics::TabButton {id:"settings-general",selected:section()=="General",label:"General",onclick:move |_|section.set("General")}
            crate::diagnostics::TabButton {id:"settings-logs",selected:section()=="Logs",label:"Logs",onclick:move |_|section.set("Logs")}
        }
        div {style:if section()=="General"{""}else{"display:none"},
        crate::server_settings::ServerSettings {project,catalog,storage_status}
        section {class:"panel",
            div {class:"panel-heading",h2 {"Appearance"}}
            div {class:"padded settings-row",
                div {h3 {"Background animation"}p {"Decorative sparks in the control panel."}}
                button {class:"effect-button","aria-pressed":effects().to_string(),onclick:move |_|effects.set(!effects()),if effects(){"Sparks: on"}else{"Sparks: off"}}
            }
        }
        section {class:"panel",
            div {class:"panel-heading",h2 {"About Acheflow"}}
            div {class:"padded",p {"Version {APP_VERSION}"}}
        }
        }
        if section()=="Logs" {crate::diagnostics::LogsPanel {project}}
    }
}
#[component]
fn Stat(title: &'static str, value: String, detail: &'static str) -> Element {
    rsx! {div {class:"stat",span {class:"stat-label","{title}"}strong {"{value}"}span {class:"stat-detail","{detail}"}}}
}
#[component]
fn SparksBackground(backend: Signal<String>) -> Element {
    let sparks = use_hook(move || CustomWidgetAttr::new(SparksWidget::new(backend)));
    rsx! {object {class:"sparks","data":sparks}}
}
