//! A distinct Keychain service: no imported workspace or guest can name this slot.
const SERVICE: &str = "Acheflow server access v1";
pub(super) fn load(server: &str) -> Result<Option<String>, String> {
    let bytes = match security_framework::passwords::get_generic_password(SERVICE, server) {
        Ok(bytes) => bytes,
        Err(error) if error.code() == -25300 => return Ok(None),
        Err(_) => return Err("Cannot read device session from Keychain.".into()),
    };
    let token = String::from_utf8(bytes).map_err(|_| "Invalid stored device session.")?;
    validate(&token)?;
    Ok(Some(token))
}
fn validate(token: &str) -> Result<(), String> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid device session token.".into());
    }
    Ok(())
}
pub(super) fn save(server: &str, token: &str) -> Result<(), String> {
    validate(token)?;
    security_framework::passwords::set_generic_password(SERVICE, server, token.as_bytes())
        .map_err(|_| "Cannot save device session in Keychain.".into())
}
pub(super) fn remove(server: &str) -> Result<(), String> {
    match security_framework::passwords::delete_generic_password(SERVICE, server) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()),
        Err(_) => Err("Cannot remove device session from Keychain.".into()),
    }
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
