use crate::integration::GuestManifest;
use serde_json::{Value, json};
fn package() -> Value {
    json!({"id":"independent","name":"Independent","api_version":2,"parts_version":1,
    "parts":{
      "client":{"kind":"provider","module":"client.wasm","entry_point":"client_handle","capabilities":["panel","http"],"services":["https://client.example"],"secret_fields":{"client_token":"/token"},"settings":[{"id":"client_setting","label":"Local"}]},
      "server":{"kind":"provider","module":"server.wasm","entry_point":"server_handle","capabilities":["chat","http"],"services":["https://server.example"],"secret_fields":{"server_token":"/token"},"settings":[{"id":"server_setting","label":"Shared"}]}
    }})
}
#[test]
fn dual_part_package_is_accepted_without_legacy_root_runtime_fields() {
    let parsed = serde_json::from_value::<GuestManifest>(package());
    assert!(parsed.is_ok(), "dual-part manifest must parse: {parsed:?}");
    parsed.unwrap().validate().unwrap();
}

#[test]
fn selected_parts_do_not_inherit_sibling_settings_credentials_or_authority() {
    use crate::parts::ExecutionHost::{Client, Server};
    let manifest: GuestManifest = serde_json::from_value(package()).unwrap();
    let client = manifest.select_part(Client).unwrap().unwrap();
    let server = manifest.select_part(Server).unwrap().unwrap();
    assert_eq!(client.module, "client.wasm");
    assert_eq!(server.module, "server.wasm");
    assert_eq!(client.entry_point, "client_handle");
    assert_eq!(server.entry_point, "server_handle");
    assert_eq!(client.settings[0].id, "client_setting");
    assert_eq!(server.settings[0].id, "server_setting");
    assert!(client.secret_fields.contains_key("client_token"));
    assert!(!client.secret_fields.contains_key("server_token"));
    assert_eq!(client.services, ["https://client.example"]);
    assert_eq!(server.services, ["https://server.example"]);
    assert!(client.parts.is_none());
    assert!(server.parts.is_none());
    assert!(client.select_part(Server).is_err());
    assert!(server.select_part(Client).is_err());
    assert!(client.require_host(Server).is_err());
    assert!(server.require_host(Client).is_err());
    assert!(manifest.require_host(Client).is_err());
}
#[test]
fn absent_part_is_not_substituted_and_legacy_eligibility_is_explicit() {
    use crate::parts::ExecutionHost::{Client, Server};
    for (missing, present) in [("client", Server), ("server", Client)] {
        let mut value = package();
        value["parts"].as_object_mut().unwrap().remove(missing);
        let parsed: GuestManifest = serde_json::from_value(value).unwrap();
        assert!(parsed.select_part(present).unwrap().is_some());
        assert!(
            parsed
                .select_part(if present == Client { Server } else { Client })
                .unwrap()
                .is_none()
        );
    }
    let legacy:GuestManifest=serde_json::from_value(json!({"id":"legacy","name":"Legacy","api_version":2,"module":"guest.wasm","execution_side":"client","capabilities":[],"services":[]})).unwrap();
    assert_eq!(
        legacy.select_part(Server).unwrap().unwrap().selected_host(),
        Some(Server)
    );
    assert_eq!(
        legacy.select_part(Client).unwrap().unwrap().entry_point,
        "handle"
    );
}
#[test]
fn ambiguous_versions_paths_and_unknown_part_fields_fail_closed() {
    for patch in [
        json!({"parts_version":2}),
        json!({"parts_version":null}),
        json!({"module":"ambiguous.wasm"}),
        json!({"capabilities":["http"]}),
    ] {
        let mut value = package();
        value
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(
            serde_json::from_value::<GuestManifest>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    for (key, value) in [
        ("module", json!("../escape.wasm")),
        ("entry_point", json!("../bad")),
        ("capabilities", json!(["unrestricted"])),
        ("supported_actions", json!(["unknown.action"])),
    ] {
        let mut v = package();
        v["parts"]["client"][key] = value;
        assert!(
            serde_json::from_value::<GuestManifest>(v)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut v = package();
    v["parts"]["client"]["credential_namespace"] = json!("server");
    assert!(serde_json::from_value::<GuestManifest>(v).is_err());
    let mut v = package();
    v["parts"] = json!({});
    assert!(
        serde_json::from_value::<GuestManifest>(v)
            .unwrap()
            .validate()
            .is_err()
    );
}
