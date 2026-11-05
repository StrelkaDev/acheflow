//! Selected fields of Google's StreamList proto; unknown fields are skipped.
//! Schema: https://developers.google.com/youtube/data/live/streaming-live-chat
use serde_json::{Value, json};
use std::collections::VecDeque;

pub struct Batch {
    pub items: VecDeque<Vec<u8>>,
    pub token: String,
    pub offline: bool,
}
enum Field<'a> {
    Number(u64),
    Data(&'a [u8]),
    Other,
}
struct Fields<'a>(&'a [u8]);
fn varint(input: &mut &[u8]) -> Result<u64, &'static str> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = input.split_first().ok_or("Truncated protobuf integer.")?;
        *input = rest;
        if shift == 63 && byte > 1 {
            return Err("Protobuf integer overflow.");
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    Err("Invalid protobuf integer.")
}
impl<'a> Fields<'a> {
    fn next(&mut self) -> Result<Option<(u64, Field<'a>)>, &'static str> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let tag = varint(&mut self.0)?;
        if tag >> 3 == 0 {
            return Err("Invalid protobuf field.");
        }
        let field = match tag & 7 {
            0 => Field::Number(varint(&mut self.0)?),
            2 => {
                let length = usize::try_from(varint(&mut self.0)?)
                    .map_err(|_| "Protobuf field exceeds memory limit.")?;
                if length > self.0.len() {
                    return Err("Truncated protobuf data.");
                }
                let (data, rest) = self.0.split_at(length);
                self.0 = rest;
                Field::Data(data)
            }
            wire @ (1 | 5) => {
                let length = if wire == 1 { 8 } else { 4 };
                if self.0.len() < length {
                    return Err("Truncated protobuf field.");
                }
                self.0 = &self.0[length..];
                Field::Other
            }
            _ => return Err("Unsupported protobuf wire type."),
        };
        Ok(Some((tag >> 3, field)))
    }
}
fn text(data: &[u8], limit: usize) -> Result<String, &'static str> {
    if data.len() > limit {
        return Err("YouTube text exceeds domain limit.");
    }
    std::str::from_utf8(data)
        .map(str::to_owned)
        .map_err(|_| "Invalid protobuf text.")
}
pub fn batch(data: &[u8]) -> Result<Batch, &'static str> {
    let mut fields = Fields(data);
    let mut batch = Batch {
        items: VecDeque::new(),
        token: String::new(),
        offline: false,
    };
    while let Some((tag, value)) = fields.next()? {
        match (tag, value) {
            (1007, Field::Data(message)) => {
                if batch.items.len() >= 2000 {
                    return Err("YouTube streaming batch exceeds 2000 messages.");
                }
                batch.items.push_back(message.to_vec());
            }
            (100602, Field::Data(token)) => {
                if token.len() > 8192 {
                    return Err("YouTube continuation exceeds limit.");
                }
                batch.token = text(token, 8192)?;
            }
            (2, Field::Data(value)) => batch.offline = !value.is_empty(),
            _ => {}
        }
    }
    Ok(batch)
}
pub fn message(data: &[u8]) -> Result<Value, &'static str> {
    let mut fields = Fields(data);
    let mut message = json!({});
    while let Some((tag, field)) = fields.next()? {
        match (tag, field) {
            (101, Field::Data(value)) => message["id"] = json!(text(value, 256)?),
            (2, Field::Data(value)) => message["snippet"] = snippet(value)?,
            (3, Field::Data(value)) => message["authorDetails"] = author(value)?,
            _ => {}
        }
    }
    if message["snippet"]["type"] == "tombstone" {
        message["snippet"]["messageDeletedDetails"] = json!({"deletedMessageId":message["id"]});
        message["snippet"]["type"] = json!("messageDeletedEvent");
    }
    Ok(message)
}
fn snippet(data: &[u8]) -> Result<Value, &'static str> {
    let mut fields = Fields(data);
    let mut result = json!({});
    while let Some((tag, field)) = fields.next()? {
        match (tag, field) {
            (1, Field::Number(kind)) => {
                result["type"] = json!(match kind {
                    2 => "tombstone",
                    4 => "chatEndedEvent",
                    _ => "message",
                })
            }
            (16, Field::Data(value)) => result["displayMessage"] = json!(text(value, 8192)?),
            (19, Field::Data(value)) => {
                let mut nested = Fields(value);
                while let Some((tag, field)) = nested.next()? {
                    if let (1, Field::Data(value)) = (tag, field) {
                        result["textMessageDetails"] = json!({"messageText":text(value,8192)?});
                    }
                }
            }
            _ => {}
        }
    }
    Ok(result)
}
fn author(data: &[u8]) -> Result<Value, &'static str> {
    let mut fields = Fields(data);
    let mut result = json!({});
    while let Some((tag, field)) = fields.next()? {
        match (tag, field) {
            (10101, Field::Data(value)) => result["channelId"] = json!(text(value, 256)?),
            (103, Field::Data(value)) => result["displayName"] = json!(text(value, 256)?),
            (5, Field::Number(value)) => result["isChatOwner"] = json!(value != 0),
            (6, Field::Number(value)) => result["isChatSponsor"] = json!(value != 0),
            (7, Field::Number(value)) => result["isChatModerator"] = json!(value != 0),
            _ => {}
        }
    }
    Ok(result)
}
fn put_integer(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    output.push(value as u8);
}
pub(crate) fn put(tag: u64, value: &[u8], output: &mut Vec<u8>) {
    put_integer(tag << 3 | 2, output);
    put_integer(value.len() as u64, output);
    output.extend_from_slice(value);
}
pub fn request(chat: &str, token: &str) -> Vec<u8> {
    let mut result = vec![];
    put(1, chat.as_bytes(), &mut result);
    for part in ["id", "snippet", "authorDetails"] {
        put(100, part.as_bytes(), &mut result);
    }
    if !token.is_empty() {
        put(99, token.as_bytes(), &mut result);
    }
    result
}
