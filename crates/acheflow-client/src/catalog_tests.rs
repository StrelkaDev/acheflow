use super::*;
use acheflow_control::extensions::Guest;

#[test]
fn bundled_duplicates_are_disabled_with_slot_diagnostics_and_no_auth_module_survivor() {
    let baseline = ClientCatalog::bundled();
    assert!(baseline.errors.is_empty(), "{:?}", baseline.errors);
    let youtube = &baseline.installed["youtube"];
    let twitch = &baseline.installed["twitch"];
    let youtube_manifest = serde_json::to_string(&youtube.manifest).unwrap();
    let twitch_manifest = serde_json::to_string(&twitch.manifest).unwrap();

    for count in [2, 3] {
        let mut installations = vec![
            (
                youtube_manifest.as_str(),
                youtube.module.as_slice(),
                Some("auth-a"),
            ),
            (twitch_manifest.as_str(), twitch.module.as_slice(), None),
            (
                youtube_manifest.as_str(),
                youtube.module.as_slice(),
                Some("auth-b"),
            ),
        ];
        if count == 3 {
            installations.push((
                youtube_manifest.as_str(),
                youtube.module.as_slice(),
                Some("auth-c"),
            ));
        }

        let catalog = ClientCatalog::from_bundled(&installations);

        assert!(
            !catalog.installed.contains_key("youtube"),
            "none of the {count} bundled installations may receive a forwarded callback"
        );
        assert_eq!(catalog.installed.len(), 1);
        let independent = &catalog.installed["twitch"];
        assert_eq!(independent.module, twitch.module);
        Guest::load(
            &independent.module,
            independent.manifest.capabilities.clone(),
        )
        .unwrap();
        assert!(independent.browser_auth.is_none());
        assert!(catalog.installed.values().all(|installation| {
            !matches!(
                installation.browser_auth.as_deref(),
                Some("auth-a" | "auth-b" | "auth-c")
            )
        }));

        let errors = catalog.errors.join("\n");
        assert!(errors.contains("youtube"), "{errors}");
        for slot in [1, 3].into_iter().chain((count == 3).then_some(4)) {
            let source = format!("bundled installation #{slot}");
            assert!(errors.contains(&source), "missing {source}: {errors}");
        }
        assert!(
            !errors.contains("bundled installation #2"),
            "the unrelated provider must not be implicated: {errors}"
        );
    }
}
