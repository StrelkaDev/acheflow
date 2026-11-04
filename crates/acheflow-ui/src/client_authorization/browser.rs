use acheflow_client::local::LocalSession;
use serde_json::Value;
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(inline_js = r#"
export async function prepare(source) {
  const url=URL.createObjectURL(new Blob([source],{type:'text/javascript'}));
  try {const module=await import(url);await module.prepare();return module;}
  finally {URL.revokeObjectURL(url);}
}
export function start(url, callback, module, clientId, scopes) {
  let popup, timer, rejectAttempt, finished=false;
  const promise=new Promise((resolve,reject)=>{
    rejectAttempt=reject;
    const accept=value=>{if(!finished){finished=true;clearInterval(timer);resolve(value);}};
    if(module) {
      // SDK must open its window synchronously from this user gesture.
      module.authorize(clientId,JSON.parse(scopes)).then(value=>accept(JSON.stringify(value)),()=>reject(new Error('Authorization interrupted')));
      timer=setTimeout(()=>reject(new Error('Authorization timed out')),300000);
      return;
    }
    popup=window.open(url,'_blank','popup,width=600,height=760');
    if(!popup){reject(new Error('Popup blocked'));return;}
    try {popup.opener=null;} catch {}
    const expected=new URL(callback), deadline=Date.now()+300000;
    timer=setInterval(()=>{
      if(Date.now()>deadline || popup.closed){clearInterval(timer);reject(new Error('Authorization cancelled or timed out'));return;}
      try {
        const current=new URL(popup.location.href);
        if(current.origin!==expected.origin || current.pathname!==expected.pathname)return;
        const fields=current.hash.slice(1)||current.search.slice(1);
        popup.history.replaceState(null,'',expected.pathname);
        popup.close();accept(fields);
      } catch { /* Provider pages are intentionally cross-origin. */ }
    },250);
  });
  return {promise,cancel(){finished=true;clearInterval(timer);if(popup&&!popup.closed)popup.close();rejectAttempt(new Error('Authorization cancelled'));}};
}
export function wait(handle) {return handle.promise;}
export function cancel(handle) {handle.cancel();}
"#)]
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
    ) -> Result<JsValue, JsValue>;
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
