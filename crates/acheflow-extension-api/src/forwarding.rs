//! Provider-neutral addressing for installed application forwarding.
//!
//! Parsing identifies a recipient; it does not authorize an action or start a
//! guest. Receivers must check manifest declarations and the live instance before
//! delivery. Callback payloads can contain credentials and must not enter logs.

use std::fmt;

pub const MAX_FORWARD_URL_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardRecipient {
    Client,
    Server,
}
impl ForwardRecipient {
    pub const fn origin(self) -> &'static str {
        match self {
            Self::Client => "https://client.acheflow.com",
            Self::Server => "https://server.acheflow.com",
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ForwardLink {
    pub recipient: ForwardRecipient,
    pub extension_id: String,
    pub action: String,
    /// Encoded path after the action; never a filesystem path.
    pub tail: Option<String>,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

impl fmt::Debug for ForwardLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForwardLink")
            .field("recipient", &self.recipient)
            .field("extension_id", &self.extension_id)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardError {
    TooLarge,
    InvalidAddress,
    WrongRecipient,
    InvalidRoute,
}
impl fmt::Display for ForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "forwarding address exceeds 16 KiB",
            Self::InvalidAddress => "invalid forwarding address",
            Self::WrongRecipient => "forwarding address belongs to another application",
            Self::InvalidRoute => "invalid extension forwarding route",
        })
    }
}
impl std::error::Error for ForwardError {}

impl ForwardLink {
    pub fn parse_for(input: &str, recipient: ForwardRecipient) -> Result<Self, ForwardError> {
        let relative = recipient_relative_address(input, recipient)?;
        let url = url::Url::parse(input).map_err(|_| ForwardError::InvalidAddress)?;
        let raw_path = relative.split(['?', '#']).next().unwrap_or_default();
        // The URL parser removes dot segments (including encoded ones). Reject
        // aliases rather than dispatching a path different from what was supplied.
        if raw_path != url.path()
            || !valid_encoding(raw_path, true)
            || !valid_encoding(url.query().unwrap_or_default(), false)
            || !valid_encoding(url.fragment().unwrap_or_default(), false)
        {
            return Err(ForwardError::InvalidRoute);
        }
        let route = raw_path
            .strip_prefix("/fwd/")
            .ok_or(ForwardError::InvalidRoute)?;
        let mut segments = route.splitn(3, '/');
        let extension_id = segments.next().unwrap_or_default();
        let action = segments.next().unwrap_or_default();
        if !crate::integration::valid_provider_id(extension_id)
            || !crate::integration::valid_id(action, 64)
            || action.contains([':', '.'])
        {
            return Err(ForwardError::InvalidRoute);
        }
        let tail = segments.next();
        if tail.is_some_and(|s| s.starts_with('/') || s.contains("//")) {
            return Err(ForwardError::InvalidRoute);
        }
        Ok(Self {
            recipient,
            extension_id: extension_id.into(),
            action: action.into(),
            tail: tail.map(str::to_owned),
            query: url.query().map(str::to_owned),
            fragment: url.fragment().map(str::to_owned),
        })
    }
}

fn recipient_relative_address(
    input: &str,
    recipient: ForwardRecipient,
) -> Result<&str, ForwardError> {
    if input.len() > MAX_FORWARD_URL_BYTES {
        return Err(ForwardError::TooLarge);
    }
    if input
        .bytes()
        .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
    {
        return Err(ForwardError::InvalidAddress);
    }
    // Require a canonical authority, excluding credentials, explicit ports,
    // suffix domains and URL-parser normalization of alternate host spellings.
    let origin = recipient.origin();
    let Some(relative) = input.strip_prefix(origin).filter(|s| s.starts_with('/')) else {
        let other = match recipient {
            ForwardRecipient::Client => ForwardRecipient::Server,
            ForwardRecipient::Server => ForwardRecipient::Client,
        };
        return Err(
            if input
                .strip_prefix(other.origin())
                .is_some_and(|s| s.starts_with('/'))
            {
                ForwardError::WrongRecipient
            } else {
                ForwardError::InvalidAddress
            },
        );
    };
    Ok(relative)
}

fn valid_encoding(text: &str, path: bool) -> bool {
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'%' {
            continue;
        }
        let Some(high) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
            return false;
        };
        let Some(low) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
            return false;
        };
        let decoded = (high * 16 + low) as u8;
        if decoded.is_ascii_control() || (path && matches!(decoded, b'/' | b'\\' | b'%')) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_provider_payload_without_interpreting_oauth_fields() {
        let link = ForwardLink::parse_for(
            "https://client.acheflow.com/fwd/provider/oauth/callback?code=a%2Bb&state=x%3Dy#opaque=secret",
            ForwardRecipient::Client,
        ).unwrap();
        assert_eq!(link.extension_id, "provider");
        assert_eq!(link.action, "oauth");
        assert_eq!(link.tail.as_deref(), Some("callback"));
        assert_eq!(link.query.as_deref(), Some("code=a%2Bb&state=x%3Dy"));
        assert_eq!(link.fragment.as_deref(), Some("opaque=secret"));
        let debug = format!("{link:?}");
        assert!(
            !debug.contains("secret") && !debug.contains("code=") && !debug.contains("callback")
        );
    }

    #[test]
    fn recipients_cannot_consume_each_others_addresses() {
        let input = "https://server.acheflow.com/fwd/example/open/settings";
        assert_eq!(
            ForwardLink::parse_for(input, ForwardRecipient::Client),
            Err(ForwardError::WrongRecipient)
        );
        let server = ForwardLink::parse_for(input, ForwardRecipient::Server).unwrap();
        assert_eq!(server.tail.as_deref(), Some("settings"));
    }

    #[test]
    fn refuses_authority_and_path_aliases() {
        for input in [
            "http://client.acheflow.com/fwd/example/open",
            "https://app.acheflow.com/fwd/example/open",
            "https://acheflow.com/fwd/example/open",
            "https://client.acheflow.com.evil/fwd/example/open",
            "https://client.acheflow.com@evil/fwd/example/open",
            "https://user@client.acheflow.com/fwd/example/open",
            "https://client.acheflow.com:443/fwd/example/open",
            "https://client.acheflow.com/link/example/open",
            "https://client.acheflow.com/fwd/%65xample/open",
            "https://client.acheflow.com/fwd/example/../other/open",
            "https://client.acheflow.com/fwd/example/%2e%2e/other/open",
            "https://client.acheflow.com/fwd/example/open/a%2Fb",
            "https://client.acheflow.com/fwd/example/open/%252e%252e",
            "https://client.acheflow.com/fwd/example/open//other",
            "https://client.acheflow.com/fwd/example/open?state=%ZZ",
            "https://client.acheflow.com/fwd/example/open#value=%00",
            "https://client.acheflow.com/fwd/example\\open",
            "https://client.acheflow.com/fwd/example/open\n",
        ] {
            assert!(
                ForwardLink::parse_for(input, ForwardRecipient::Client).is_err(),
                "accepted {input:?}"
            );
        }
    }

    #[test]
    fn requires_bounded_manifest_id_and_action() {
        for route in [
            "",
            "example",
            "/open",
            "example/",
            "ui/open",
            "acheflow/open",
            "a.b/open",
            "example/a:b",
        ] {
            assert!(
                ForwardLink::parse_for(
                    &format!("https://client.acheflow.com/fwd/{route}"),
                    ForwardRecipient::Client
                )
                .is_err()
            );
        }
        let too_long = format!(
            "https://client.acheflow.com/fwd/example/open?{}",
            "a".repeat(MAX_FORWARD_URL_BYTES)
        );
        assert_eq!(
            ForwardLink::parse_for(&too_long, ForwardRecipient::Client),
            Err(ForwardError::TooLarge)
        );
        assert!(
            ForwardLink::parse_for(
                "https://client.acheflow.com/fwd/example/open",
                ForwardRecipient::Client
            )
            .is_ok()
        );
    }
}
