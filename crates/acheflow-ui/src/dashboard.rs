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
    let monitor_tab = use_signal(|| true);
    let telemetry_interval = use_memo(move || match page() {
        "Overview" => Some(250),
        "Live" if monitor_tab() => Some(100),
        _ => None,
    });
    use_context_provider(|| crate::live_client::LiveTelemetryInterval(telemetry_interval));
    let (project, mut catalog, storage_status) = use_workspaces();
    use_context_provider(|| catalog);
    crate::server_settings::provide(project, catalog);
    crate::server_access::provide(project);
    let effects = use_signal(|| catalog.peek().background_animation());
    use_effect(move || {
        let enabled = effects();
        if catalog.peek().background_animation() != enabled {
            catalog.write().set_background_animation(enabled);
        }
    });
    let workspace_open = use_signal(|| false);
    let live = use_live_client(project);
    crate::audio_monitor::provide(project, catalog);
    let control = crate::control_client::use_control_client(project);
    let sync = crate::workspace_sync::provide(project, catalog, control);
    crate::diagnostics::observe(live, control);
    crate::interaction::use_chat_draft(project);
    #[cfg(not(target_arch = "wasm32"))]
    let activation_notice = crate::native_activation::provide(project, page);
    rsx! {
        {text_input_mount()}
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
                SidebarFooter {backend,following:(sync.following)(),ready:(sync.ready)()}
            }
            div {class:"workspace",
                {native_activation_notice(
                    #[cfg(not(target_arch = "wasm32"))]
                    activation_notice,
                )}
                if effects(){SparksBackground {backend}}
                Workspaces {project,catalog,storage_status,open:workspace_open}
                crate::workspace_sync::Notice {}
                div {class:if live.running(){"workspace-body with-live-chat"}else{"workspace-body"},
                div {class:"tab-viewport",for tab in ["Overview","Live","Extensions","Settings"] {
                    div {key:"{tab}",class:if page()==tab{"tab-scroll active"}else{"tab-scroll"},"aria-hidden":if page()==tab{"false"}else{"true"},
                        if tab=="Live"{Live {key:"{catalog.read().active_id()}",project,visible:page()=="Live",monitor_tab}}else{
                            main {class:"content",
                                if tab=="Overview"{
                                    div {class:"page-heading",div {h1 {"Overview"}p {"Your broadcast workspace, in one place."}}span {class:"badge",if live.running(){"Running"}else{"Idle"}}}
                                    div {class:"stats",
                                        Stat {title:"EXPECTED INPUTS",value:project.read().sources.len().to_string(),detail:"RTMP sources"}
                                        Stat {title:"SCENES",value:project.read().scenes.len().to_string(),detail:"Horizontal + vertical"}
                                        Stat {title:"RESTREAM OUTPUTS",value:(project.read().horizontal.outputs.len()+project.read().vertical.outputs.len()).to_string(),detail:"Configured destinations"}
                                        Stat {title:"SESSION",value:if live.running(){"Running"}else{"Idle"},detail:"Media worker state"}
                                    }
                                    if page()=="Overview" {crate::live::ServerLoadPanel {}}
                                    section {class:"panel",div {class:"panel-heading",h2 {"Prepare your broadcast"}}div {class:"padded",h3 {"Sources → Scenes → Studio"}p {"Register expected RTMP inputs in Live / Sources. Build compositions in Live / Scenes, set separate output profiles in Settings and monitor both formats in Studio."}p {"FFmpeg ingests authorized RTMP sources and renders scenes. Manage the live session in Studio."}}}
                                }

                                if tab=="Extensions"{
                                    crate::interaction::ExtensionsPage {}
                                }
                                if tab=="Settings"{GeneralSettings {effects,project,catalog,storage_status,visible:page()=="Settings"}}
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

#[component]
fn SidebarFooter(backend: Signal<String>, following: bool, ready: bool) -> Element {
    let status = match (following, ready) {
        (true, true) => "Server workspace",
        (true, false) => "Connecting to workspace",
        (false, _) => "Local draft",
    };
    rsx! {
        div {class:"sidebar-footer",
            div {class:"local-badge",span {class:"status-dot"},span {"{status}"}}
            p {class:"sidebar-backend","{backend}"}
            p {class:"app-version","v{APP_VERSION}"}
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
    #[props(default = true)] visible: bool,
) -> Element {
    let mut section = use_signal(|| "General");
    rsx! {
        div {class:"page-heading",div {h1 {"Settings"}p {"Server connection, desktop behavior and appearance."}}}
        nav {class:"settings-tabs","aria-label":"Settings sections",
            crate::diagnostics::TabButton {id:"settings-general",selected:section()=="General",label:"General",onclick:move |_|section.set("General")}
            crate::diagnostics::TabButton {id:"settings-access",selected:section()=="Access",label:"Device access",onclick:move |_|section.set("Access")}
            crate::diagnostics::TabButton {id:"settings-logs",selected:section()=="Logs",label:"Logs",onclick:move |_|section.set("Logs")}
        }
        div {style:if section()=="General"{""}else{"display:none"},
        crate::server_settings::ServerSettings {project,catalog,storage_status}
        if visible && section()=="General" {crate::audio_monitor::OutputSettings {}}
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
        if section()=="Access" {crate::server_access::Panel {project}}
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

fn text_input_mount() -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        rsx! {crate::mobile_input::Mount {}}
    }
    #[cfg(target_os = "macos")]
    {
        rsx! {crate::native_text_input::Mount {}}
    }
    #[cfg(not(any(target_os = "macos", target_arch = "wasm32")))]
    {
        rsx! {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_dom::{Document, DocumentConfig};
    use blitz_traits::shell::Viewport;
    use std::{cell::Cell, rc::Rc};

    #[tokio::test]
    async fn sidebar_footer_keeps_its_rows_when_the_server_connects() {
        type Ready = Rc<Cell<Option<Signal<bool>>>>;
        fn app() -> Element {
            let ready = use_signal(|| false);
            use_context::<Ready>().set(Some(ready));
            let backend = use_signal(|| "Metal / Blitz + wgpu".into());
            rsx! {
                style {{include_str!("dashboard.css")}}
                div {class:"app",aside {class:"sidebar",SidebarFooter {backend,following:true,ready:ready()}}}
            }
        }
        let ready = Ready::default();
        let mut doc = dioxus_native::DioxusDocument::new(
            VirtualDom::new(app).with_root_context(ready.clone()),
            DocumentConfig {
                font_ctx: Some(dioxus_native::build_single_font_ctx(include_bytes!(
                    "../assets/DejaVuSans.woff2"
                ))),
                ..Default::default()
            },
        );
        doc.initial_build();
        doc.inner.borrow_mut().set_viewport(Viewport {
            window_size: (1280, 900),
            ..Default::default()
        });
        let rows = |doc: &dioxus_native::DioxusDocument| {
            let dom = doc.inner.borrow();
            [".local-badge", ".sidebar-backend", ".app-version"].map(|selector| {
                let id = dom.query_selector(selector).unwrap().unwrap();
                let node = dom.get_node(id).unwrap();
                (
                    node.absolute_position(0., 0.).y,
                    node.final_layout().size.height,
                )
            })
        };
        doc.inner.borrow_mut().resolve(0.);
        let connecting = rows(&doc);
        ready.get().unwrap().set(true);
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.);
        {
            let dom = doc.inner.borrow();
            let id = dom.query_selector(".local-badge").unwrap().unwrap();
            assert_eq!(dom.get_node(id).unwrap().text_content(), "Server workspace");
        }
        assert_eq!(rows(&doc), connecting);
        for pair in connecting.windows(2) {
            assert!(
                pair[0].1 > 0. && pair[0].0 + pair[0].1 < pair[1].0,
                "footer rows should have a positive height and gap: {connecting:?}"
            );
        }
    }
}
