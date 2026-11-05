//! Packaged web assets use a fixed sibling directory; explicit overrides win.
use std::path::{Path, PathBuf};

pub fn web_root() -> PathBuf {
    if let Some(root) = std::env::var_os("ACHEFLOW_WEB_ROOT") {
        return root.into();
    }
    default_web_root(
        PathBuf::from("dist"),
        std::env::current_exe().ok().as_deref(),
    )
}

fn default_web_root(local: PathBuf, executable: Option<&Path>) -> PathBuf {
    if local.is_dir() {
        return local;
    }
    executable
        .and_then(Path::parent)
        .and_then(|directory| {
            [directory.join("dist"), directory.join("../Resources/dist")]
                .into_iter()
                .find(|root| root.is_dir())
        })
        .unwrap_or(local)
}

#[cfg(test)]
mod tests {
    #[test]
    fn web_assets_are_found_beside_an_installed_server() {
        let root = std::env::temp_dir().join(format!("af-assets-{}", std::process::id()));
        let web = root.join("dist");
        std::fs::create_dir_all(&web).unwrap();
        let local = root.join("working-directory/dist");
        let binary = root.join("acheflow-server");
        assert_eq!(super::default_web_root(local.clone(), Some(&binary)), web);
        std::fs::create_dir_all(&local).unwrap();
        assert_eq!(super::default_web_root(local.clone(), Some(&binary)), local);
        std::fs::remove_dir_all(root).unwrap();
    }
}
