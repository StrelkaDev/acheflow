//! Independent provider-shaped wire fixtures for actual WASM integration tests.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
fn integer(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    output.push(value as u8);
}
fn field(tag: u64, value: &[u8], output: &mut Vec<u8>) {
    integer((tag << 3) | 2, output);
    integer(value.len() as u64, output);
    output.extend_from_slice(value);
}
pub fn batch(count: usize, text_len: usize, token: &str) -> Vec<u8> {
    let mut output = vec![];
    for n in 0..count {
        let mut snippet = vec![8, 1];
        field(
            16,
            format!("Message {n}: {}", "x".repeat(text_len)).as_bytes(),
            &mut snippet,
        );
        let mut author = vec![56, 1];
        field(10101, b"user", &mut author);
        field(103, b"User", &mut author);
        let mut message = vec![];
        field(101, format!("m{n}").as_bytes(), &mut message);
        field(2, &snippet, &mut message);
        field(3, &author, &mut message);
        field(1007, &message, &mut output);
    }
    field(100602, token.as_bytes(), &mut output);
    output
}
pub fn event(id: &Value, bytes: &[u8], end: bool) -> Value {
    json!({"type":"http_stream_data","id":id,"data":STANDARD.encode(bytes),"end_message":end,"now_ms":1000})
}
pub fn decoded_request(effect: &Value) -> Vec<u8> {
    STANDARD
        .decode(effect["body_base64"].as_str().unwrap())
        .unwrap()
}
