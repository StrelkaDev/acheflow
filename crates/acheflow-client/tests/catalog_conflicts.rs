#![cfg(not(target_arch = "wasm32"))]

use acheflow_client::catalog::ClientCatalog;
use acheflow_control::extensions::Guest;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Installations(PathBuf);

impl Installations {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "acheflow-client-catalog-conflicts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn install(&self, directory: &str, provider: &str, id: &str, browser_source: Option<&str>) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions")
            .join(provider);
        let destination = self.0.join(directory);
        std::fs::create_dir(&destination).unwrap();
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(source.join("manifest.json")).unwrap()).unwrap();
        manifest["id"] = id.into();
        let module = manifest["module"].as_str().unwrap();
        std::fs::copy(source.join(module), destination.join(module)).unwrap();
        if let Some(name) = manifest["authentication"]["browser_module"].as_str() {
            match browser_source {
                Some(source) => std::fs::write(destination.join(name), source).unwrap(),
                None => {
                    std::fs::copy(source.join(name), destination.join(name)).unwrap();
                }
            }
        }
        std::fs::write(
            destination.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Installations {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn two_conflicting_client_installations_are_both_disabled_and_report_their_sources() {
    let root = Installations::new();
    root.install("a-twitch", "twitch", "twitch", None);
    root.install("b-copy", "twitch", "twitch", None);
    root.install("c-youtube", "youtube", "youtube", None);

    let catalog = ClientCatalog::discover_at(&root.0);

    assert!(
        !catalog.installed.contains_key("twitch"),
        "a conflicting ID must have no eligible recipient"
    );
    let youtube = &catalog.installed["youtube"];
    Guest::load(&youtube.module, youtube.manifest.capabilities.clone()).unwrap();
    let errors = catalog.errors.join("\n");
    assert!(errors.contains("twitch"), "{errors}");
    for directory in ["a-twitch", "b-copy"] {
        assert!(
            errors.contains(directory),
            "missing source {directory}: {errors}"
        );
    }
}

#[test]
fn a_third_duplicate_cannot_restore_a_conflicted_client_id() {
    let root = Installations::new();
    for directory in ["a-first", "b-second", "c-third"] {
        root.install(directory, "twitch", "twitch", None);
    }
    root.install("d-independent", "vklive", "vklive", None);

    let catalog = ClientCatalog::discover_at(&root.0);

    assert!(!catalog.installed.contains_key("twitch"));
    assert!(catalog.installed.contains_key("vklive"));
    let errors = catalog.errors.join("\n");
    for directory in ["a-first", "b-second", "c-third"] {
        assert!(
            errors.contains(directory),
            "missing source {directory}: {errors}"
        );
    }
}

#[test]
fn a_rejected_duplicate_cannot_replace_another_installations_browser_module() {
    let root = Installations::new();
    root.install("a-original", "youtube", "youtube", Some("original-auth"));
    root.install("b-duplicate", "youtube", "youtube", Some("duplicate-auth"));
    root.install(
        "c-independent",
        "youtube",
        "independent",
        Some("independent-auth"),
    );

    let catalog = ClientCatalog::discover_at(&root.0);

    assert!(
        !catalog.installed.contains_key("youtube"),
        "conflicting browser-auth modules cannot have an active installation"
    );
    assert_eq!(
        catalog.installed["independent"].browser_auth.as_deref(),
        Some("independent-auth")
    );
    assert!(
        catalog
            .installed
            .values()
            .all(|installation| installation.browser_auth.as_deref() != Some("duplicate-auth"))
    );
}

#[test]
fn the_catalog_limit_cannot_hide_a_late_duplicate_of_an_accepted_id() {
    let root = Installations::new();
    for index in 0..8 {
        root.install(
            &format!("a-{index}"),
            "twitch",
            &format!("provider-{index}"),
            None,
        );
    }
    root.install("z-duplicate", "twitch", "provider-0", None);

    let catalog = ClientCatalog::discover_at(&root.0);

    assert!(!catalog.installed.contains_key("provider-0"));
    for index in 1..8 {
        assert!(catalog.installed.contains_key(&format!("provider-{index}")));
    }
    assert_eq!(catalog.installed.len(), 7);
}

#[test]
fn an_incomplete_client_directory_scan_cannot_publish_a_partial_catalog() {
    let root = Installations::new();
    root.install("valid-provider", "twitch", "twitch", None);
    for index in 0..65 {
        std::fs::create_dir(root.0.join(format!("extra-{index}"))).unwrap();
    }

    let catalog = ClientCatalog::discover_at(&root.0);

    assert!(
        catalog.installed.is_empty(),
        "an unscanned entry could hide a conflicting installation"
    );
    assert!(
        catalog
            .errors
            .iter()
            .any(|error| error.contains("scan limit")),
        "{:?}",
        catalog.errors
    );
}
