use acheflow_media_api::server::TlsFiles;
use axum_server::tls_openssl::OpenSSLConfig;
use openssl::{
    pkey::PKey,
    ssl::{SslAcceptor, SslMethod},
};

pub fn load(files: &TlsFiles) -> Result<Option<OpenSSLConfig>, String> {
    if files.cert.is_empty() && files.key.is_empty() {
        return Ok(None);
    }
    let key = std::fs::read(&files.key)
        .map_err(|e| format!("Cannot read private key {}: {e}", files.key))?;
    // An explicit empty passphrase rejects encrypted keys without a terminal prompt.
    let build = || -> Result<OpenSSLConfig, openssl::ssl::Error> {
        let key = PKey::private_key_from_pem_passphrase(&key, b"")?;
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())?;
        builder.set_certificate_chain_file(&files.cert)?;
        builder.set_private_key(&key)?;
        OpenSSLConfig::try_from(builder)
    };
    build().map(Some).map_err(|e| {
        format!(
            "Invalid TLS certificate/key pair ({}); use an unencrypted PEM private key: {e}",
            files.cert
        )
    })
}
