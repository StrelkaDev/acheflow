//! Device sessions stay separate from workspace provider credentials.
pub(super) fn load(server: &str) -> Result<Option<String>, String> {
    crate::vault::device_session(server, None)
}
fn validate(token: &str) -> Result<(), String> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid device session token.".into());
    }
    Ok(())
}
pub(super) fn save(server: &str, token: &str) -> Result<(), String> {
    validate(token)?;
    crate::vault::device_session(server, Some(Some(token)))?;
    Ok(())
}
pub(super) fn remove(server: &str) -> Result<(), String> {
    crate::vault::device_session(server, Some(None))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an unlocked macOS login keychain; run explicitly"]
    fn real_device_session_cannot_be_read_through_provider_storage() {
        let mut bytes = [0; 16];
        getrandom::getrandom(&mut bytes).unwrap();
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let server = format!("https://device-vault-{id}.example");
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = remove(&self.0);
            }
        }
        let _cleanup = Cleanup(server.clone());
        let token = "f".repeat(64);
        save(&server, &token).unwrap();
        assert!(load(&server).unwrap().as_deref() == Some(token.as_str()));
        let provider =
            crate::vault::key(&server, "server-access", "acheflow-device", "session").unwrap();
        assert!(crate::vault::load(&provider).unwrap().is_none());
        remove(&server).unwrap();
        assert!(load(&server).unwrap().is_none());
    }
}
