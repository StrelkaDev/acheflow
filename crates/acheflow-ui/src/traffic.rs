use acheflow_media_api::{live::LiveStatus, workspace::Workspace};
use dioxus_native::prelude::*;
use std::collections::{BTreeMap, VecDeque};
use web_time::Instant;

const WINDOW: f64 = 120.;
const COLORS: [&str; 9] = [
    "#e9edf5", "#ff975c", "#69c5e8", "#94ce91", "#e6c36b", "#dca0cc", "#a5b9fa", "#67cfbf",
    "#ef9898",
];
#[derive(Clone, Default, PartialEq)]
struct Sample {
    at: f64,
    rates: BTreeMap<String, f64>,
}
#[derive(Clone, PartialEq)]
pub struct History {
    started: Instant,
    samples: VecDeque<Sample>,
    labels: BTreeMap<String, String>,
}
impl Default for History {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            samples: VecDeque::new(),
            labels: BTreeMap::new(),
        }
    }
}
impl History {
    pub fn record(&mut self, state: &LiveStatus, workspace: &Workspace) {
        if state.workspace_id.as_deref() != Some(&workspace.id) {
            *self = Self::default();
            return;
        }
        let mut sample = Sample {
            at: self.started.elapsed().as_secs_f64(),
            ..Default::default()
        };
        for source in &state.sources {
            let key = format!("in:{}", source.id);
            self.labels.insert(
                key.clone(),
                workspace
                    .sources
                    .iter()
                    .find(|s| s.id == source.id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| format!("Source {}", source.id)),
            );
            sample.rates.insert(
                key,
                if source.connected {
                    source.bitrate_kbps
                } else {
                    0.
                },
            );
        }
        for format in &state.formats {
            for output in &format.outputs {
                let key = format!("out:{}:{}", format.orientation.label(), output.index);
                self.labels.insert(
                    key.clone(),
                    format!("{} · {}", output.name, format.orientation.label()),
                );
                sample.rates.insert(key, output.bitrate_kbps);
            }
        }
        for prefix in ["in:", "out:"] {
            let total = sample
                .rates
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(_, v)| v)
                .sum();
            sample.rates.insert(format!("{prefix}total"), total);
        }
        self.push(sample);
    }
    fn push(&mut self, sample: Sample) {
        let cutoff = sample.at - WINDOW;
        self.samples.push_back(sample);
        while self.samples.front().is_some_and(|s| s.at < cutoff) || self.samples.len() > 240 {
            self.samples.pop_front();
        }
        self.labels
            .retain(|key, _| self.samples.iter().any(|s| s.rates.contains_key(key)));
    }
    fn latest(&self, key: &str) -> f64 {
        self.samples
            .back()
            .and_then(|s| s.rates.get(key))
            .copied()
            .unwrap_or(0.)
    }
    fn path(&self, key: &str, ceiling: f64) -> String {
        use std::fmt::Write;
        let end = self.samples.back().map(|s| s.at).unwrap_or(WINDOW);
        let mut path = String::new();
        let mut previous = None;
        for sample in &self.samples {
            if let Some(rate) = sample.rates.get(key) {
                let command = if previous.is_some_and(|at| sample.at - at < 3.) {
                    'L'
                } else {
                    'M'
                };
                let x = 52. + ((sample.at - (end - WINDOW)) / WINDOW).clamp(0., 1.) * 728.;
                let y = 180. - (rate / ceiling).clamp(0., 1.) * 156.;
                let _ = write!(path, "{command}{x:.1},{y:.1} ");
                previous = Some(sample.at);
            } else {
                previous = None;
            }
        }
        path
    }
}
#[component]
pub fn TrafficCharts() -> Element {
    let client = use_context::<crate::live_client::LiveClient>();
    let history = client.traffic.read();
    let receiving = history.latest("in:total") / 1000.;
    let sending = history.latest("out:total") / 1000.;
    rsx! {
        div {class:"stats traffic-stats",
            div {class:"stat",span {class:"stat-label","TOTAL RECEIVING"}strong {"{receiving:.2} Mbps"}span {class:"stat-detail","All incoming sources"}}
            div {class:"stat",span {class:"stat-label","TOTAL SENDING"}strong {"{sending:.2} Mbps"}span {class:"stat-detail","All outgoing destinations"}}
        }
        div {class:"traffic-charts",for (prefix,title) in [("in:","Receiving"),("out:","Sending")] {
            TrafficChart {history:history.clone(),prefix,title}
        }}
        p {class:"form-help","Last 2 minutes · rolling media bitrate, including audio. Incoming: compressed media payload; outgoing: encoded FLV bytes. Excludes network protocol overhead and preview traffic."}
    }
}
#[component]
fn TrafficChart(history: History, prefix: &'static str, title: &'static str) -> Element {
    let total_key = format!("{prefix}total");
    let mut series = vec![(total_key.clone(), "Total".to_owned())];
    series.extend(
        history
            .labels
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    let ceiling = history
        .samples
        .iter()
        .flat_map(|s| s.rates.iter())
        .filter(|(k, _)| k.starts_with(prefix))
        .map(|(_, v)| *v)
        .fold(1000., f64::max)
        * 1.1;
    rsx! {section {class:"panel traffic-panel",
        div {class:"panel-heading",h2 {"{title}"}span {class:"badge","Mbps"}}
        div {class:"traffic-plot",div {class:"traffic-canvas",
            svg {view_box:"0 0 800 212",role:"img","aria-label":format!("{title} bitrate over the last two minutes"),
                for fraction in [0.,0.5,1.] {
                    line {x1:"52",x2:"780",y1:format!("{}",180.-fraction*156.),y2:format!("{}",180.-fraction*156.),stroke:"#303b4e",stroke_width:"1"}
                }
                for (i,(key,_)) in series.iter().enumerate() {
                    path {d:history.path(key,ceiling),fill:"none",stroke:COLORS[i%COLORS.len()],stroke_width:if i==0 {"3"}else{"2"},stroke_dasharray:if i==0 {"6 3"}else{"none"}}
                }
            }
            for fraction in [0.,0.5,1.] {
                span {class:"traffic-axis-label",style:format!("top:{}%;",(180.-fraction*156.)/212.*100.),"{ceiling*fraction/1000.:.1}"}
            }
            span {class:"traffic-time-start","−2 min"}span {class:"traffic-time-end","Now"}
        }}
        div {class:"traffic-legend",for (i,(key,label)) in series.iter().enumerate() {
            div {class:"traffic-series",span {class:"traffic-swatch",style:format!("background:{}",COLORS[i%COLORS.len()])}span {"{label}"}strong {"{history.latest(key)/1000.:.2} Mbps"}}
        }}
        if history.samples.is_empty(){p {class:"padded form-help","Waiting for server measurements…"}}
    }}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totals_include_each_stream_and_history_cannot_cross_workspaces() {
        let workspace = Workspace::default();
        let state: LiveStatus=serde_json::from_value(serde_json::json!({
            "workspace_id":workspace.id,"rtmp_address":"127.0.0.1:1935",
            "sources":[
                {"id":1,"connected":true,"frames":10,"width":640,"height":360,"fps":30.,"audio":true,"error":null,"bitrate_kbps":1500.},
                {"id":2,"connected":true,"frames":10,"width":640,"height":360,"fps":30.,"audio":true,"error":null,"bitrate_kbps":2500.}
            ],
            "formats":[{"orientation":"horizontal","frames":20,"outputs":[
                {"index":1,"name":"Same name","state":"streaming","frames":20,"error":null,"bitrate_kbps":1000.},
                {"index":2,"name":"Same name","state":"streaming","frames":20,"error":null,"bitrate_kbps":2000.}
            ]}]
        })).unwrap();
        let mut h = History::default();
        h.record(&state, &workspace);
        assert_eq!(h.latest("in:total"), 4000.);
        assert_eq!(h.latest("out:total"), 3000.);
        assert_eq!(h.labels.len(), 4);
        h.record(&state, &Workspace::default());
        assert!(h.samples.is_empty());
        assert!(h.labels.is_empty());
    }
    #[test]
    fn newest_measurement_is_at_now_even_before_the_history_window_fills() {
        let mut h = History::default();
        h.push(Sample {
            at: 1.,
            rates: BTreeMap::from([("in:1".into(), 500.)]),
        });
        h.push(Sample {
            at: 2.,
            rates: BTreeMap::from([("in:1".into(), 500.)]),
        });
        assert!(h.path("in:1", 1000.).ends_with("L780.0,102.0 "));
    }
    #[test]
    fn graph_keeps_a_bounded_window_and_breaks_lines_on_missing_samples() {
        let mut h = History::default();
        for (at, present) in [(0., true), (1., true), (2., false), (3., true), (10., true)] {
            h.push(Sample {
                at,
                rates: if present {
                    BTreeMap::from([("in:1".into(), 500.)])
                } else {
                    BTreeMap::new()
                },
            });
        }
        let path = h.path("in:1", 1000.);
        assert_eq!(path.matches('M').count(), 3);
        assert_eq!(path.matches('L').count(), 1);
        for at in 11..400 {
            h.push(Sample {
                at: at as f64,
                ..Default::default()
            });
        }
        assert!(h.samples.len() <= 121);
        assert!(h.samples.front().unwrap().at >= 279.);
        assert_eq!(h.latest("in:1"), 0.);
    }
}
