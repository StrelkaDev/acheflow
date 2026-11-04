use super::*;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Installations(PathBuf);

impl Installations {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "acheflow-server-catalog-conflicts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn install(&self, directory: &str, id: &str) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/sample");
        let destination = self.0.join(directory);
        std::fs::create_dir(&destination).unwrap();
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(source.join("manifest.json")).unwrap()).unwrap();
        manifest["id"] = id.into();
        let module = manifest["module"].as_str().unwrap();
        std::fs::copy(source.join(module), destination.join(module)).unwrap();
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
fn server_duplicate_installations_are_all_disabled_and_report_their_sources() {
    for count in [2, 3] {
        let root = Installations::new();
        for index in 0..count {
            root.install(&format!("duplicate-{index}"), "sample");
        }
        root.install("independent", "independent");

        let catalog = discover_catalog_at(&root.0);

        assert!(
            !catalog.installed.contains_key("sample"),
            "none of the {count} conflicting installations may be active"
        );
        let independent = &catalog.installed["independent"];
        Guest::load(
            &independent.module,
            independent.manifest.capabilities.clone(),
        )
        .unwrap();
        let errors = catalog.errors.join("\n");
        assert!(errors.contains("sample"), "{errors}");
        for index in 0..count {
            let directory = format!("duplicate-{index}");
            assert!(errors.contains(&directory), "missing {directory}: {errors}");
        }
    }
}

#[test]
fn the_server_catalog_limit_cannot_hide_a_conflicting_id() {
    let root = Installations::new();
    for index in 0..8 {
        root.install(&format!("a-{index}"), &format!("logic-{index}"));
    }
    root.install("z-duplicate", "logic-0");

    let catalog = discover_catalog_at(&root.0);

    assert!(!catalog.installed.contains_key("logic-0"));
    for index in 1..8 {
        assert!(catalog.installed.contains_key(&format!("logic-{index}")));
    }
    assert_eq!(catalog.installed.len(), 7);
}

#[test]
fn an_incomplete_server_directory_scan_cannot_publish_a_partial_catalog() {
    let root = Installations::new();
    root.install("valid-provider", "sample");
    for index in 0..65 {
        std::fs::create_dir(root.0.join(format!("extra-{index}"))).unwrap();
    }

    let catalog = discover_catalog_at(&root.0);

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
