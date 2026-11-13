use acheflow_extension_api::visual::{SourceDocument, SourceNode, SourceType};
use serde_json::json;
#[test]
fn visual_source_documents_are_bounded_inert_data() {
    let document: SourceDocument = serde_json::from_value(json!({"css":".message {color:white}","nodes":[{"tag":"div","classes":"message entering","text":"<script> & \"message\"","animation_at_ms":1000}]})).unwrap();
    document.validate().unwrap();
    let mut oversized = document.clone();
    oversized.nodes[0].text = "x".repeat(262145);
    assert!(oversized.validate().is_err());
    let mut bad_class = document;
    bad_class.nodes[0].classes = "message' onclick='x".into();
    assert!(bad_class.validate().is_err());
    assert!(serde_json::from_value::<SourceNode>(json!({"tag":"script","text":"x"})).is_err());
}
#[test]
fn source_types_have_local_stable_identities() {
    let mut source: SourceType =
        serde_json::from_value(json!({"id":"clock","name":"Clock"})).unwrap();
    source.validate().unwrap();
    source.id = "other:clock".into();
    assert!(source.validate().is_err());
}
