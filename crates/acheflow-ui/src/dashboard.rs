use crate::{
    live::Live,
    live_client::use_live_client,
    sparks::SparksWidget,
    workspaces::{Workspaces, use_workspaces},
};
use acheflow_extension_api::ExtensionPanel;
use dioxus_native::{CustomWidgetAttr, prelude::*};

pub fn dashboard() -> Element {
    let mut effects = use_signal(|| true);
    let backend = use_signal(|| "Starting GPU...".to_owned());
    let mut page = use_signal(|| "Overview");
    let (project, catalog, storage_status) = use_workspaces();
    let live = use_live_client(project);
    let extension_panels = use_signal(Vec::<ExtensionPanel>::new);
    rsx! {
        style {{include_str!("dashboard.css")}}
        div {class:"app",
            aside {class:"sidebar",
                div {class:"brand",span {class:"brand-mark","a"}"acheflow"}
                div {class:"workspace-label","WORKSPACE"}
                div {class:"workspace-name",span {class:"workspace-icon","A"}"{project.read().name}"}
                nav {for item in ["Overview","Live","Extensions"] {
                    button {class:if page()==item{"nav-item selected"}else{"nav-item"},onclick:move |_|page.set(item),
                        span {class:"nav-symbol",if item=="Live"{"▷"}else{"▦"}}"{item}"
                    }
                }}
                div {class:"sidebar-footer",div {class:"local-badge",span {class:"status-dot"}"Local workspace"}p {"Acheflow / Preview"}}
            }
            div {class:"workspace",
                if effects(){SparksBackground {backend}}
                header {class:"topbar",
                    div {class:"breadcrumb","Workspace"span {"/"}strong {"{page}"}}
                    div {class:"top-actions",span {class:"preview-badge","PREVIEW"}button {class:"effect-button",onclick:move |_|effects.set(!effects()),if effects(){"Sparks: on"}else{"Sparks: off"}}}
                }
                Workspaces {project,catalog,storage_status}
                div {class:"tab-viewport",for tab in ["Overview","Live","Extensions"] {
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
                                    div {class:"page-heading",h1 {"Extensions"}}
                                    if extension_panels.read().is_empty(){section {class:"panel empty-state",h3 {"No extensions loaded"}p {"The WASM plugin runtime is not connected yet."}}}
                                    for panel in extension_panels.read().iter(){{crate::extension_panel::render(panel)}}
                                }
                                footer {class:"content-footer",span {"ACHEFLOW / STREAM CONTROL"}span {"{backend}"}}
                            }
                        }
                    }
                }}
            }
        }
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
