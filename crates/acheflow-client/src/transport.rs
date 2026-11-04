//! Bounded provider-neutral transport. Protocols and OAuth live in guests.
use acheflow_control::extensions::GuestManifest;
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

pub type Secrets = BTreeMap<String, String>;
pub fn allowed_url(
    raw: &str,
    manifest: &GuestManifest,
    websocket: bool,
) -> Result<url::Url, String> {
    let url = url::Url::parse(raw).map_err(|_| "invalid extension service URL")?;
    if !manifest
        .services
        .contains(&url.origin().ascii_serialization())
        || url.scheme() != if websocket { "wss" } else { "https" }
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("forbidden: extension service is not granted".into());
    }
    Ok(url)
}
#[cfg(not(target_arch = "wasm32"))]
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HTTP client initialization failed".into())
}
pub(crate) fn resolve(value: &Value, secrets: &Secrets) -> Result<Value, String> {
    fn charge(remaining: &mut usize, size: usize) -> Result<(), String> {
        if size > *remaining {
            return Err("resolved extension request exceeds 1 MiB".into());
        }
        *remaining -= size;
        Ok(())
    }
    fn text_size(value: &str) -> usize {
        2 + value
            .bytes()
            .map(|b| match b {
                b'"' | b'\\' => 2,
                0..=31 => 6,
                _ => 1,
            })
            .sum::<usize>()
    }
    fn visit(value: &Value, secrets: &Secrets, remaining: &mut usize) -> Result<Value, String> {
        match value {
            Value::Object(map)
                if map.contains_key("secret")
                    && map
                        .keys()
                        .all(|key| matches!(key.as_str(), "secret" | "prefix")) =>
            {
                let key = map["secret"].as_str().ok_or("invalid secret handle")?;
                let secret = secrets.get(key).ok_or("secret handle is not available")?;
                let prefix = match map.get("prefix") {
                    Some(value) => value.as_str().ok_or("invalid secret prefix")?,
                    None => "",
                };
                if prefix.len() > 128 {
                    return Err("secret prefix too long".into());
                }
                charge(remaining, text_size(prefix) + text_size(secret) - 2)?;
                Ok(json!(format!("{prefix}{secret}")))
            }
            Value::Object(map) => {
                charge(remaining, 2)?;
                let mut result = serde_json::Map::new();
                for (key, value) in map {
                    charge(remaining, text_size(key) + 2)?;
                    result.insert(key.clone(), visit(value, secrets, remaining)?);
                }
                Ok(Value::Object(result))
            }
            Value::Array(values) => {
                charge(remaining, 2)?;
                let mut result = vec![];
                for value in values {
                    charge(remaining, 1)?;
                    result.push(visit(value, secrets, remaining)?);
                }
                Ok(Value::Array(result))
            }
            Value::String(value) => {
                charge(remaining, text_size(value))?;
                Ok(json!(value))
            }
            _ => {
                charge(remaining, 32)?;
                Ok(value.clone())
            }
        }
    }
    visit(value, secrets, &mut (1024 * 1024))
}
pub(crate) fn validate_header(name: &str) -> Result<(), String> {
    if matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "connection"
            | "upgrade"
            | "content-length"
            | "transfer-encoding"
            | "proxy-authorization"
            | "proxy-connection"
            | "sec-websocket-key"
            | "sec-websocket-version"
    ) {
        return Err("forbidden: extension cannot override transport headers".into());
    }
    Ok(())
}
pub fn capture(payload: &mut Value, paths: &Value, secrets: &mut Secrets) -> Result<(), String> {
    if let Some(paths) = paths.as_object() {
        if paths.len() > 32 {
            return Err("too many secret capture paths".into());
        }
        for (key, path) in paths {
            if !acheflow_extension_api::integration::valid_id(key, 64) {
                return Err("invalid secret handle".into());
            }
            let path = path
                .as_str()
                .filter(|p| p.starts_with('/') && p.len() <= 256)
                .ok_or("invalid secret capture path")?;
            if let Some(value) = payload.pointer_mut(path) {
                if let Some(secret) = value.as_str() {
                    if secret.len() > 8192 || (!secrets.contains_key(key) && secrets.len() >= 32) {
                        return Err("secret store limit reached".into());
                    }
                    secrets.insert(key.clone(), secret.into());
                }
                *value = Value::Null;
            }
        }
    }
    Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
pub async fn http(
    effect: &Value,
    manifest: &GuestManifest,
    secrets: &mut Secrets,
) -> Result<Value, String> {
    let url = allowed_url(
        effect["url"].as_str().ok_or("missing service URL")?,
        manifest,
        false,
    )?;
    let method = match effect["method"].as_str().unwrap_or("GET") {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PATCH" => reqwest::Method::PATCH,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        _ => return Err("unsupported extension HTTP method".into()),
    };
    let request = resolve(effect, secrets)?;
    let mut builder = client()?.request(method, url);
    if let Some(headers) = request["headers"].as_object() {
        if headers.len() > 32 {
            return Err("too many request headers".into());
        }
        for (key, value) in headers {
            validate_header(key)?;
            builder = builder.header(key, value.as_str().ok_or("invalid request header")?);
        }
    }
    if let Some(query) = request["query"].as_object() {
        builder = builder.query(query);
    }
    if let Some(form) = request["form"].as_object() {
        builder = builder.form(form);
    } else if !request["body"].is_null() {
        builder = builder.json(&request["body"]);
    }
    let mut response = builder
        .send()
        .await
        .map_err(|_| "platform request interrupted")?;
    let status = response.status().as_u16();
    let retry = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "platform response interrupted")?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("platform response exceeds 1 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut payload = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "invalid platform JSON response")?
    };
    // Captured credentials are removed even from error responses before guest delivery.
    capture(
        &mut payload,
        &serde_json::to_value(&manifest.secret_fields)
            .map_err(|_| "invalid secret capture declaration")?,
        secrets,
    )?;
    capture(&mut payload, &effect["capture"], secrets)?;
    Ok(
        json!({"type":"http_result","id":effect["id"],"status":status,"payload":payload,"retry_after":retry}),
    )
}
#[cfg(target_arch = "wasm32")]
pub async fn http(
    effect: &Value,
    manifest: &GuestManifest,
    secrets: &mut Secrets,
) -> Result<Value, String> {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(inline_js = r#"
    export async function local_request(encoded) {
      const e=JSON.parse(encoded), url=new URL(e.url);
      for(const [k,v] of Object.entries(e.query||{}))url.searchParams.set(k,String(v));
      const headers=new Headers(e.headers||{}), options={method:e.method||'GET',headers,redirect:'error',credentials:'omit',cache:'no-store',signal:AbortSignal.timeout(10000)};
      if(e.form){options.body=new URLSearchParams(e.form);headers.set('Content-Type','application/x-www-form-urlencoded');}
      else if(e.body!=null){options.body=JSON.stringify(e.body);headers.set('Content-Type','application/json');}
      const response=await fetch(url,options), reader=response.body?.getReader();
      let bytes=0, parts=[];
      if(reader)try {for(;;){const {done,value}=await reader.read();if(done)break;bytes+=value.length;if(bytes>1048576){await reader.cancel();throw new Error('Response too large');}parts.push(value);}}finally{reader.releaseLock();}
      const all=new Uint8Array(bytes);let offset=0;for(const part of parts){all.set(part,offset);offset+=part.length;}
      const text=new TextDecoder().decode(all);
      return JSON.stringify({type:'http_result',id:e.id,status:response.status,payload:text?JSON.parse(text):null,retry_after:response.headers.get('retry-after')});
    }
    "#)]
    extern "C" {
        #[wasm_bindgen(catch)]
        fn local_request(encoded: &str) -> Result<js_sys::Promise, JsValue>;
    }
    allowed_url(
        effect["url"].as_str().ok_or("Missing service URL.")?,
        manifest,
        false,
    )?;
    if !matches!(
        effect["method"].as_str().unwrap_or("GET"),
        "GET" | "POST" | "PATCH" | "PUT" | "DELETE"
    ) {
        return Err("Unsupported HTTP method.".into());
    }
    let request = resolve(effect, secrets)?;
    if let Some(headers) = request["headers"].as_object() {
        if headers.len() > 32 {
            return Err("Too many headers.".into());
        }
        for name in headers.keys() {
            validate_header(name)?;
        }
    }
    let promise = local_request(&serde_json::to_string(&request).map_err(|_| "Invalid request.")?)
        .map_err(|_| "Platform request interrupted.")?;
    let result = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| "Platform request interrupted.")?;
    let mut result: Value = serde_json::from_str(&result.as_string().ok_or("Invalid response.")?)
        .map_err(|_| "Invalid response.")?;
    capture(
        &mut result["payload"],
        &serde_json::to_value(&manifest.secret_fields)
            .map_err(|_| "Invalid credential mapping.")?,
        secrets,
    )?;
    capture(&mut result["payload"], &effect["capture"], secrets)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_are_redacted_and_secret_handles_are_scoped_to_the_calling_store() {
        let mut secrets = Secrets::new();
        let mut value =
            json!({"token":"private","nested":{"refresh":"private-refresh"},"public":"name"});
        capture(
            &mut value,
            &json!({"access":"/token","refresh":"/nested/refresh"}),
            &mut secrets,
        )
        .unwrap();
        assert!(value["token"].is_null());
        assert!(value["nested"]["refresh"].is_null());
        assert_eq!(value["public"], "name");
        let header = json!({"secret":"access","prefix":"Bearer "});
        assert_eq!(resolve(&header, &secrets).unwrap(), "Bearer private");
        assert!(resolve(&header, &Secrets::new()).is_err());
    }
    #[test]
    fn origins_and_schemes_are_explicit_and_do_not_follow_provider_names() {
        let manifest:GuestManifest=serde_json::from_value(json!({"id":"fixture","name":"Fixture","api_version":2,"module":"guest.wasm","execution_side":"client","capabilities":["http"],"services":["https://provider.example"]})).unwrap();
        assert!(allowed_url("https://provider.example/data", &manifest, false).is_ok());
        for url in [
            "https://other.example/data",
            "https://user:pass@provider.example/data",
            "http://provider.example/data",
            "https://provider.example/data#secret",
        ] {
            assert!(allowed_url(url, &manifest, false).is_err());
        }
    }
    #[test]
    fn declared_secret_limit_and_substitution_budget_are_enforced_consistently() {
        let paths: serde_json::Map<String, Value> = (0..32)
            .map(|n| (format!("secret{n}"), json!(format!("/field{n}"))))
            .collect();
        let mut payload: Value = Value::Object(
            (0..32)
                .map(|n| (format!("field{n}"), json!("private")))
                .collect(),
        );
        let mut secrets = Secrets::new();
        capture(&mut payload, &Value::Object(paths), &mut secrets).unwrap();
        assert_eq!(secrets.len(), 32);
        assert!(payload.as_object().unwrap().values().all(Value::is_null));
        let secrets = Secrets::from([("large".into(), "x".repeat(8192))]);
        let repeated = Value::Array(vec![json!({"secret":"large"}); 256]);
        assert!(resolve(&repeated, &secrets).unwrap_err().contains("1 MiB"));
    }
    #[tokio::test]
    async fn a_guest_cannot_override_the_destination_host_header() {
        let manifest:GuestManifest=serde_json::from_value(json!({"id":"fixture","name":"Fixture","api_version":2,"module":"guest.wasm","execution_side":"client","capabilities":["http"],"services":["https://provider.example"]})).unwrap();
        let error = http(
            &json!({"url":"https://provider.example/api","headers":{"Host":"other.example"}}),
            &manifest,
            &mut Secrets::new(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("transport headers"));
    }
}
