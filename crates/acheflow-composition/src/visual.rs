//! Render only inert source nodes. Message text never becomes markup or attributes.
use acheflow_media_api::appearance::validate_css;
use acheflow_media_api::visual::{SourceDocument, SourceNode};
pub(crate) fn source_html(
    document: &SourceDocument,
    epoch_ms: u64,
    now_ms: u64,
) -> Result<String, String> {
    document.validate()?;
    validate_css(&document.css)?;
    fn escaped(text: &str, out: &mut String) {
        for c in text.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                c => out.push(c),
            }
        }
    }
    fn node(value: &SourceNode, epoch: u64, now: u64, out: &mut String) {
        if value.remove_at_ms.is_some_and(|at| now >= at) {
            return;
        }
        let leaving = value.exit_at_ms.is_some_and(|at| now >= at);
        let tag = value.tag.html();
        out.push('<');
        out.push_str(tag);
        out.push_str(" class=\"");
        escaped(&value.classes, out);
        if value.animation_at_ms.is_some() {
            out.push_str(if leaving { " leaving" } else { " entering" });
        }
        out.push('"');
        if let Some(at) = if leaving {
            value.exit_at_ms
        } else {
            value.animation_at_ms
        } {
            out.push_str(&format!(
                " style=\"animation-delay:{:.3}s\"",
                (at as f64 - epoch as f64) / 1000.
            ));
        }
        out.push('>');
        escaped(&value.text, out);
        for child in &value.children {
            node(child, epoch, now, out);
        }
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
    let mut html = String::new();
    for value in &document.nodes {
        node(value, epoch_ms, now_ms, &mut html);
    }
    Ok(html)
}

pub(crate) fn phases(document: &SourceDocument, now: u64) -> Vec<u8> {
    fn walk(nodes: &[SourceNode], now: u64, out: &mut Vec<u8>) {
        for node in nodes {
            out.push(if node.remove_at_ms.is_some_and(|at| now >= at) {
                2
            } else if node.exit_at_ms.is_some_and(|at| now >= at) {
                1
            } else {
                0
            });
            walk(&node.children, now, out);
        }
    }
    let mut output = vec![];
    walk(&document.nodes, now, &mut output);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_nodes_escape_chat_and_keep_absolute_animation_timing() {
        let document = SourceDocument {
            nodes: vec![SourceNode {
                text: "<script>&\"'".into(),
                animation_at_ms: Some(1250),
                ..Default::default()
            }],
            ..Default::default()
        };
        let html = source_html(&document, 1000, 1000).unwrap();
        assert!(html.contains("&lt;script&gt;&amp;&quot;&#39;"));
        assert!(html.contains("animation-delay:0.250s"));
        assert!(!html.contains("<script>"));
    }
}
