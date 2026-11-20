//! Production server, real TLS, browser cookies and independently granted devices.
//! All accounts, certificates, providers and chat messages are disposable fixtures.
use acheflow_media_api::{
    access::{Invitation, PairResponse, Permission as P, Session},
    control::ControlSnapshot,
    server_workspace::{ImportWorkspace, WorkspaceState},
    spatial::SpatialScene,
    workspace::{Orientation, Workspace},
};
use openssl::{
    asn1::Asn1Time,
    bn::BigNum,
    hash::MessageDigest,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{
        X509, X509NameBuilder,
        extension::{BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAlternativeName},
    },
};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::process::{Child, Command};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "acheflow-shared-https-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn certificate(key: &PKey<Private>, issuer: Option<(&X509, &PKey<Private>)>) -> X509 {
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text(
        "CN",
        if issuer.is_some() {
            "localhost"
        } else {
            "Disposable test CA"
        },
    )
    .unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    let serial = BigNum::from_u32(if issuer.is_some() { 2 } else { 1 })
        .unwrap()
        .to_asn1_integer()
        .unwrap();
    cert.set_serial_number(&serial).unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(issuer.map(|(ca, _)| ca.subject_name()).unwrap_or(&name))
        .unwrap();
    cert.set_pubkey(key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    if let Some((ca, _)) = issuer {
        cert.append_extension(BasicConstraints::new().critical().build().unwrap())
            .unwrap();
        cert.append_extension(
            KeyUsage::new()
                .critical()
                .digital_signature()
                .key_encipherment()
                .build()
                .unwrap(),
        )
        .unwrap();
        cert.append_extension(ExtendedKeyUsage::new().server_auth().build().unwrap())
            .unwrap();
        let san = SubjectAlternativeName::new()
            .dns("localhost")
            .ip("127.0.0.1")
            .build(&cert.x509v3_context(Some(ca), None))
            .unwrap();
        cert.append_extension(san).unwrap();
    } else {
        cert.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        cert.append_extension(
            KeyUsage::new()
                .critical()
                .key_cert_sign()
                .crl_sign()
                .build()
                .unwrap(),
        )
        .unwrap();
    }
    cert.sign(
        issuer.map(|(_, key)| key).unwrap_or(key),
        MessageDigest::sha256(),
    )
    .unwrap();
    cert.build()
}

struct Server {
    directory: Directory,
    client: Client,
    base: String,
    http: u16,
    rtmp: u16,
    child: Option<Child>,
}
impl Server {
    async fn start() -> Self {
        let directory = Directory::new();
        let ca_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let ca = certificate(&ca_key, None);
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let leaf = certificate(&key, Some((&ca, &ca_key)));
        std::fs::write(
            directory.0.join("cert.pem"),
            [leaf.to_pem().unwrap(), ca.to_pem().unwrap()].concat(),
        )
        .unwrap();
        let key_path = directory.0.join("key.pem");
        std::fs::write(&key_path, key.private_key_to_pem_pkcs8().unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        // Real WASM collectors use no platform network or OS credential storage.
        for provider in ["alpha", "beta"] {
            let root = directory.0.join("extensions").join(provider);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("fixture.wasm"),
                include_bytes!("fixtures/domain.wasm"),
            )
            .unwrap();
            let manifest = json!({"id":provider,"name":provider,"api_version":2,"parts_version":1,"parts":{"server":{"kind":"provider","module":"fixture.wasm","entry_point":"handle","capabilities":["channel","chat","delivery","status"],"settings":[{"id":"server_fixture","label":"Fixture"}],"supported_actions":["chat.send"]}}});
            std::fs::write(
                root.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }
        let free_port = || {
            std::net::TcpListener::bind(("127.0.0.1", 0))
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        };
        let http = free_port();
        let mut rtmp = free_port();
        while rtmp == http {
            rtmp = free_port();
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .add_root_certificate(reqwest::Certificate::from_pem(&ca.to_pem().unwrap()).unwrap())
            .build()
            .unwrap();
        let mut server = Self {
            directory,
            client,
            base: format!("https://localhost:{http}"),
            http,
            rtmp,
            child: None,
        };
        server.launch().await;
        server
    }
    async fn launch(&mut self) {
        let log = std::fs::File::create(self.directory.0.join("server.log")).unwrap();
        self.child = Some(
            Command::new(env!("CARGO_BIN_EXE_acheflow-server"))
                .arg("--config")
                .arg(self.directory.0.join("server.json"))
                .args([
                    "--listen",
                    &format!("127.0.0.1:{}", self.http),
                    "--rtmp-listen",
                    &format!("127.0.0.1:{}", self.rtmp),
                    "--http-cert",
                ])
                .arg(self.directory.0.join("cert.pem"))
                .arg("--http-key")
                .arg(self.directory.0.join("key.pem"))
                .env(
                    "ACHEFLOW_EXTENSIONS_DIR",
                    self.directory.0.join("extensions"),
                )
                .env_remove("ACHEFLOW_UI_ORIGINS")
                .env_remove("ACHEFLOW_MCP_TOKEN")
                .env("ACHEFLOW_MCP_PORT", "0")
                .current_dir(&self.directory.0)
                .stdin(Stdio::piped())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if self
                .client
                .get(format!("{}/healthz", self.base))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "HTTPS server exited before readiness: {}",
                std::fs::read_to_string(self.directory.0.join("server.log")).unwrap_or_default()
            );
            assert!(
                tokio::time::Instant::now() < deadline,
                "HTTPS server did not become ready"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    async fn stop(&mut self) {
        let mut child = self.child.take().unwrap();
        child.start_kill().unwrap();
        tokio::time::timeout(Duration::from_secs(15), child.wait())
            .await
            .unwrap()
            .unwrap();
    }
    fn owner(&self) -> Device {
        let record: Value = serde_json::from_slice(
            &std::fs::read(self.directory.0.join("server.access.json")).unwrap(),
        )
        .unwrap();
        Device {
            credential: Credential::Bearer(record["owner_key"].as_str().unwrap().into()),
        }
    }
    async fn ready_chat(&self, device: &Device) -> ControlSnapshot {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot: ControlSnapshot = device
                .request(self, Method::GET, "/api/control/state")
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            if snapshot
                .server_providers
                .iter()
                .filter(|p| p.state == "running")
                .count()
                == 2
                && ["alpha", "beta"].iter().all(|id| {
                    snapshot
                        .chat
                        .entries
                        .iter()
                        .any(|m| m.message.platform == *id)
                })
            {
                return snapshot;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "Both shared collectors must become ready"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

// No Debug: browser cookies and native bearer credentials must not enter logs.
enum Credential {
    Bearer(String),
    Cookie(String),
}
struct Device {
    credential: Credential,
}
impl Device {
    fn request(&self, server: &Server, method: Method, path: &str) -> reqwest::RequestBuilder {
        let request = server
            .client
            .request(method, format!("{}{path}", server.base))
            .header("x-acheflow-request", "1");
        match &self.credential {
            Credential::Bearer(token) => request.bearer_auth(token),
            Credential::Cookie(cookie) => request
                .header("cookie", cookie)
                .header("origin", &server.base),
        }
    }
    async fn invite(&self, server: &Server, permissions: &[P], linked: bool) -> Self {
        let path = if linked {
            "/api/access/link-invitations"
        } else {
            "/api/access/invitations"
        };
        let invitation: Invitation = self
            .request(server, Method::POST, path)
            .json(&json!({"permissions":permissions}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let response = server
            .client
            .post(format!("{}/api/access/pair", server.base))
            .header("origin", &server.base)
            .header("x-acheflow-request", "1")
            .json(&json!({"code":invitation.code,"name":"Fixture browser"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .to_owned();
        for flag in [
            "__Host-acheflow_session=",
            "Path=/",
            "HttpOnly",
            "Secure",
            "SameSite=Strict",
        ] {
            assert!(cookie.contains(flag));
        }
        assert!(!cookie.contains("Domain="));
        let response: PairResponse = response.json().await.unwrap();
        assert!(
            response.token.is_none(),
            "Browser pairing must not expose its bearer token"
        );
        Self {
            credential: Credential::Cookie(cookie.split(';').next().unwrap().into()),
        }
    }
    async fn session(&self, server: &Server) -> Session {
        self.request(server, Method::GET, "/api/access/session")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn trusted_https_devices_share_read_only_collectors_with_independent_roles_and_durable_sessions()
 {
    let mut server = Server::start().await;
    // A fresh untrusted client rejects our fixture CA: no certificate bypasses.
    assert!(
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("{}/healthz", server.base))
            .send()
            .await
            .is_err()
    );
    let owner = server.owner();
    let mut workspace = Workspace::default();
    workspace.add_source();
    workspace.sources[0].key = "private-ingest-fixture".into();
    let scene = workspace.add_scene(Orientation::Horizontal);
    for provider in ["alpha", "beta"] {
        workspace.server_integrations.insert(
            provider.into(),
            acheflow_media_api::server_provider::ServerIntegration {
                enabled: true,
                application: "fixture".into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                config: [("server_fixture".into(), "true".into())].into(),
            },
        );
        workspace.client_capabilities.push(
            acheflow_extension_api::client_execution::CapabilityGrant {
                provider: provider.into(),
                account_id: "bot".into(),
                channel_id: "channel".into(),
                capabilities: [acheflow_extension_api::client_execution::Capability::ChatRead]
                    .into(),
            },
        );
    }
    let state: WorkspaceState = owner
        .request(&server, Method::POST, "/api/workspace/import")
        .json(&ImportWorkspace {
            expected_revision: 0,
            workspace,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace = state.workspace.unwrap();
    assert!(
        owner
            .request(&server, Method::PUT, "/api/live/config")
            .json(&workspace)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let initial = server.ready_chat(&owner).await;
    let viewer = owner
        .invite(
            &server,
            &[P::StudioView, P::InteractiveView, P::ChatRead],
            false,
        )
        .await;
    let sender = owner
        .invite(&server, &[P::ChatRead, P::ChatSend], true)
        .await;
    let linked_reader = sender.invite(&server, &[P::ChatRead], true).await;
    let operator = owner
        .invite(&server, &[P::StudioView, P::ScenesManage], false)
        .await;
    let admin = owner.invite(&server, &P::ALL, false).await;
    let sender_identity = sender.session(&server).await;
    let reader_identity = linked_reader.session(&server).await;
    assert_eq!(sender_identity.user.id, reader_identity.user.id);
    assert_ne!(sender_identity.device.id, reader_identity.device.id);
    assert!(!reader_identity.device.permissions.contains(&P::ChatSend));
    assert_ne!(
        viewer.session(&server).await.user.id,
        sender_identity.user.id
    );
    let view = viewer
        .request(&server, Method::GET, "/api/workspace")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let view: Value = serde_json::from_str(&view).unwrap();
    assert!(view["workspace"].is_null());
    assert_eq!(view["view"]["sources"][0]["key"], "");
    let permission = json!({"workspace_id":workspace.id,"provider":"alpha","channel_id":"channel","operation":"chat.send"});
    assert_eq!(
        sender
            .request(&server, Method::POST, "/api/access/actions/authorize")
            .json(&permission)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for device in [&viewer, &linked_reader, &operator] {
        assert_eq!(
            device
                .request(&server, Method::POST, "/api/access/actions/authorize")
                .json(&permission)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            device
                .request(&server, Method::GET, "/api/extensions/server-providers")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            device
                .request(&server, Method::POST, "/api/extensions/packages/stage")
                .body("invalid fixture archive")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let snapshot: ControlSnapshot = operator
        .request(&server, Method::GET, "/api/control/state")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let command = json!({"workspace_id":workspace.id,"request_id":format!("{}:scene-fixture",std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()),"expected_revision":snapshot.revision,"action":{"type":"set_scene","scene_id":scene,"spatial":SpatialScene::from_canvas(&workspace.horizontal.base)}});
    assert_eq!(
        viewer
            .request(&server, Method::POST, "/api/control/commands")
            .json(&command)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let edited = operator
        .request(&server, Method::POST, "/api/control/commands")
        .json(&command)
        .send()
        .await
        .unwrap();
    let status = edited.status();
    let body = edited.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "Scene operator response: {body}");
    assert_eq!(
        admin
            .request(&server, Method::GET, "/api/access/devices")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let mut cross_site = admin
        .request(&server, Method::POST, "/api/access/link-invitations")
        .header("x-forwarded-proto", "https")
        .json(&json!({"permissions":[P::ChatRead]}))
        .build()
        .unwrap();
    cross_site
        .headers_mut()
        .insert("origin", "https://foreign.example".parse().unwrap());
    assert_eq!(
        server.client.execute(cross_site).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let after = server.ready_chat(&owner).await;
    assert_eq!(after.server_providers.len(), initial.server_providers.len());
    assert_eq!(after.channels.len(), 2);
    assert!(
        after
            .channels
            .iter()
            .all(|c| c.capabilities == ["chat.read"])
    );
    let viewer_id = viewer.session(&server).await.device.id;
    assert_eq!(
        owner
            .request(
                &server,
                Method::DELETE,
                &format!("/api/access/devices/{viewer_id}")
            )
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        viewer
            .request(&server, Method::GET, "/api/control/state")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    server.stop().await;
    server.launch().await;
    assert_eq!(
        sender.session(&server).await.device.id,
        sender_identity.device.id
    );
    let restored = server.ready_chat(&owner).await;
    assert_eq!(restored.server_providers.len(), 2);
    assert_eq!(restored.channels.len(), 2);
    assert_eq!(
        sender
            .request(&server, Method::POST, "/api/access/actions/authorize")
            .json(&permission)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        viewer
            .request(&server, Method::GET, "/api/control/state")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    server.stop().await;
}
