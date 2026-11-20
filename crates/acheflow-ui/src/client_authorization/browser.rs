use acheflow_client::local::LocalSession;
use serde_json::Value;
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/client_authorization/browser_auth.js")]
extern "C" {
    #[wasm_bindgen(catch,js_name=prepare)]
    fn prepare_js(source: &str) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch,js_name=start)]
    fn start_js(
        url: &str,
        callback: &str,
        module: &JsValue,
        client_id: &str,
        scopes: &str,
        proxy: &str,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name=loadRoute)]
    pub fn load_route() -> bool;
    #[wasm_bindgen(js_name=saveRoute)]
    pub fn save_route(hosted: bool);
    fn wait(handle: &JsValue) -> js_sys::Promise;
    fn cancel(handle: &JsValue);
}
pub async fn prepare(source: &str) -> Result<JsValue, JsValue> {
    JsFuture::from(prepare_js(source)?).await
}
pub struct Login {
    handle: JsValue,
    sdk: bool,
}
impl Drop for Login {
    fn drop(&mut self) {
        cancel(&self.handle);
    }
}
pub fn start_effect(
    session: &LocalSession,
    effects: &[Value],
    module: Option<JsValue>,
    environment: &Value,
) -> Result<Option<Login>, String> {
    let Some(effect) = effects
        .iter()
        .find(|e| matches!(e["type"].as_str(), Some("browser" | "browser_module")))
    else {
        return Ok(None);
    };
    let callback = environment["callback_uri"]
        .as_str()
        .ok_or("Missing local callback.")?;
    let sdk = effect["type"] == "browser_module";
    if !sdk && effect["callback_uri"].as_str() != Some(callback) {
        return Err("The extension callback does not match the selected return route.".into());
    }
    if sdk && module.is_none() {
        return Err(
            "The platform sign-in library is still loading. Try again shortly or enter a token."
                .into(),
        );
    }
    let module = if sdk {
        module.unwrap_or(JsValue::NULL)
    } else {
        JsValue::NULL
    };
    let url = if sdk {
        String::new()
    } else {
        acheflow_client::transport::allowed_url(
            effect["url"].as_str().ok_or("Missing browser URL.")?,
            &session.manifest,
            false,
        )?
        .to_string()
    };
    let handle = start_js(
        &url,
        callback,
        &module,
        effect["arguments"]["client_id"]
            .as_str()
            .unwrap_or_default(),
        &effect["arguments"]["scopes"].to_string(),
        environment["app_callback_uri"]
            .as_str()
            .unwrap_or("https://app.acheflow.com/callback/"),
    )
    .map_err(|_| "Could not start platform authorization.")?;
    Ok(Some(Login { handle, sdk }))
}
impl Login {
    pub async fn finish(self) -> Result<Value, String> {
        let value = JsFuture::from(wait(&self.handle)).await.map_err(
            |_| "Authorization cancelled, denied or timed out. Allow sign-in popups and try again.",
        )?;
        let value = value.as_string().ok_or("Invalid authorization response.")?;
        if self.sdk {
            serde_json::from_str(&value).map_err(|_| "Invalid platform response.".into())
        } else {
            if value.len() > 16384 {
                return Err("Callback too large.".into());
            }
            let mut fields = BTreeMap::new();
            for (key, value) in url::form_urlencoded::parse(value.as_bytes()) {
                if fields
                    .insert(key.into_owned(), value.into_owned())
                    .is_some()
                {
                    return Err("Duplicate callback field.".into());
                }
            }
            serde_json::to_value(fields).map_err(|_| "Invalid callback.".into())
        }
    }
}
