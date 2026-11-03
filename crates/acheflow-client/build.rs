//! Bundle client installations for the browser; provider definitions remain in extensions/.
use std::{env, fs, path::PathBuf};
fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../extensions");
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.toml").display()
    );
    let mut manifests: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path().join("manifest.json"))
        .filter(|p| p.is_file())
        .collect();
    manifests.sort();
    let mut generated = String::from("pub const BUNDLED: &[(&str, &[u8])] = &[\n");
    let mut count = 0;
    for path in manifests {
        println!("cargo:rerun-if-changed={}", path.display());
        assert!(
            fs::metadata(&path).unwrap().len() <= 65536,
            "client manifest exceeds 64 KiB"
        );
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if value["execution_side"] != "client" {
            continue;
        }
        count += 1;
        assert!(count <= 8, "too many bundled client extensions");
        let module = value["module"].as_str().unwrap();
        assert!(!module.contains(['/', '\\']) && module.ends_with(".wasm"));
        let module = path.parent().unwrap().join(module).canonicalize().unwrap();
        assert!(
            module.starts_with(path.parent().unwrap().canonicalize().unwrap()),
            "client module escapes installation"
        );
        assert!(
            fs::metadata(&module).unwrap().len() <= 4 * 1024 * 1024,
            "client module exceeds 4 MiB"
        );
        println!("cargo:rerun-if-changed={}", module.display());
        generated.push_str(&format!(
            "(include_str!({:?}), include_bytes!({:?})),\n",
            path.canonicalize().unwrap(),
            module.canonicalize().unwrap()
        ));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("client_extensions.rs"),
        generated,
    )
    .unwrap();
}
