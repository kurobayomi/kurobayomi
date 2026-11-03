//! PKCE verifier and the browser authorise URL (`PkceUtil.kt:8-14`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::TryRng;
use rand::rngs::SysRng;
use url::Url;

use super::consts::OAUTH_BASE;

/// Random bytes behind the verifier (`PkceUtil.kt:9`). 50 bytes encode to 67 unpadded base64url characters, the
/// length upstream sends.
const VERIFIER_BYTES: usize = 50;

/// A new verifier from the OS random source, base64url without padding.
pub fn new_verifier() -> String {
    let mut bytes = [0u8; VERIFIER_BYTES];
    SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS random source is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The authorise URL with the keys in source order (`client_id`, `code_challenge`, `response_type`). The
/// challenge is the verifier itself (plain method). No `redirect_uri` is sent, MAL uses the registered one.
pub fn authorize_url(client_id: &str, verifier: &str) -> Url {
    let mut url =
        Url::parse(&format!("{OAUTH_BASE}/authorize")).expect("constant base is a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("code_challenge", verifier)
        .append_pair("response_type", "code");
    url
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn pkce_verifier_shape() {
        let v = new_verifier();
        assert_eq!(v.len(), 67);
        assert!(
            v.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert!(!v.contains('='));
    }

    #[test]
    fn pkce_verifier_unique() {
        let set: HashSet<String> = (0..1000).map(|_| new_verifier()).collect();
        assert_eq!(set.len(), 1000);
    }

    #[test]
    fn authorize_url_order() {
        let url = authorize_url("cid", "abc");
        assert_eq!(
            url.as_str(),
            "https://myanimelist.net/v1/oauth2/authorize?client_id=cid&code_challenge=abc&response_type=code"
        );
        assert!(
            url.query_pairs()
                .all(|(k, _)| k != "redirect_uri" && k != "code_challenge_method")
        );
    }
}
