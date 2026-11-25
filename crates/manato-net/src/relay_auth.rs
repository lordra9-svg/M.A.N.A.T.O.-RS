//! TURN-style relay authentication.
//!
//! Two modes are supported:
//! - shared secret: HMAC-SHA256(secret, `username || 0 || nonce`),
//! - long-term credentials: realm + user/password, where the key is
//!   MD5(`username:realm:password`) and the request MAC is
//!   HMAC-SHA256(key, `username || 0 || realm || 0 || nonce`). This follows
//!   TURN's long-term credential idea (using SHA-256 instead of the legacy
//!   HMAC-SHA1 message integrity).

use std::collections::{HashMap, HashSet};

use bincode::Options as _;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const AUTH_JOIN: &[u8] = b"MANATO_RELAY_AUTH_JOIN_V1:";
const MAX_USERNAME: usize = 32;
const MAX_REALM: usize = 64;
const MAX_SEEN: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub realm: String,
    pub nonce: u64,
    pub mac: [u8; 32],
}

impl AuthRequest {
    pub fn new(secret: &str, username: &str, nonce: u64) -> Self {
        Self {
            username: username.to_string(),
            realm: String::new(),
            nonce,
            mac: shared_mac(secret, username, nonce),
        }
    }

    /// Creates a long-term-credential request.
    pub fn new_long_term(realm: &str, username: &str, password: &str, nonce: u64) -> Self {
        Self {
            username: username.to_string(),
            realm: realm.to_string(),
            nonce,
            mac: long_term_mac(realm, username, password, nonce),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = AUTH_JOIN.to_vec();
        let body = bincode::serialize(self).unwrap_or_default();
        out.extend_from_slice(&body);
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let body = bytes.strip_prefix(AUTH_JOIN)?;
        if body.len() > 512 {
            return None;
        }
        // `encode` uses fixed-int bincode. Limit decoding to the frame size so
        // a hostile length prefix cannot force a large allocation.
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(body.len() as u64)
            .deserialize(body)
            .ok()
    }
}

fn shared_mac(secret: &str, username: &str, nonce: u64) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key");
    mac.update(username.as_bytes());
    mac.update(&[0]);
    mac.update(&nonce.to_be_bytes());
    mac.finalize().into_bytes().into()
}

fn long_term_key(realm: &str, username: &str, password: &str) -> Vec<u8> {
    let mut hasher = Md5::new();
    hasher.update(format!("{username}:{realm}:{password}").as_bytes());
    hasher.finalize().to_vec()
}

fn long_term_mac(realm: &str, username: &str, password: &str, nonce: u64) -> [u8; 32] {
    let key = long_term_key(realm, username, password);
    let mut mac = HmacSha256::new_from_slice(&key).expect("hmac accepts any key");
    mac.update(username.as_bytes());
    mac.update(&[0]);
    mac.update(realm.as_bytes());
    mac.update(&[0]);
    mac.update(&nonce.to_be_bytes());
    mac.finalize().into_bytes().into()
}

enum AuthMode {
    Shared(String),
    LongTerm {
        realm: String,
        users: HashMap<String, String>,
    },
}

/// Relay-side validator with replay protection.
pub struct RelayAuthenticator {
    mode: AuthMode,
    seen: HashSet<(String, u64)>,
}

impl RelayAuthenticator {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            mode: AuthMode::Shared(secret.into()),
            seen: HashSet::new(),
        }
    }

    pub fn new_long_term(
        realm: impl Into<String>,
        users: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            mode: AuthMode::LongTerm {
                realm: realm.into(),
                users: users.into_iter().collect(),
            },
            seen: HashSet::new(),
        }
    }

    pub fn validate(&mut self, request: &AuthRequest) -> bool {
        let username = request.username.trim();
        if username.is_empty()
            || username.chars().count() > MAX_USERNAME
            || username != request.username
            || request.realm.chars().count() > MAX_REALM
        {
            return false;
        }
        let valid = match &self.mode {
            AuthMode::Shared(secret) => {
                let mut mac =
                    HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key");
                mac.update(username.as_bytes());
                mac.update(&[0]);
                mac.update(&request.nonce.to_be_bytes());
                mac.verify_slice(&request.mac).is_ok()
            }
            AuthMode::LongTerm { realm, users } => {
                if &request.realm != realm {
                    return false;
                }
                let Some(password) = users.get(username) else {
                    return false;
                };
                let key = long_term_key(realm, username, password);
                let mut mac = HmacSha256::new_from_slice(&key).expect("hmac accepts any key");
                mac.update(username.as_bytes());
                mac.update(&[0]);
                mac.update(realm.as_bytes());
                mac.update(&[0]);
                mac.update(&request.nonce.to_be_bytes());
                mac.verify_slice(&request.mac).is_ok()
            }
        };
        if !valid {
            return false;
        }
        if self.seen.contains(&(username.to_string(), request.nonce)) {
            return false;
        }
        if self.seen.len() >= MAX_SEEN {
            self.seen.clear();
        }
        self.seen.insert((username.to_string(), request.nonce));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_auth() -> RelayAuthenticator {
        RelayAuthenticator::new_long_term("manato", [("player".to_string(), "pass".to_string())])
    }

    #[test]
    fn shared_secret_validates_and_rejects_replay() {
        let mut auth = RelayAuthenticator::new("s3cret");
        let request = AuthRequest::new("s3cret", "player", 7);
        let decoded = AuthRequest::decode(&request.encode()).expect("decode");
        assert!(auth.validate(&decoded));
        assert!(!auth.validate(&decoded));
        assert!(auth.validate(&AuthRequest::new("s3cret", "player", 8)));
    }

    #[test]
    fn long_term_credentials_validate_realm_and_password() {
        let mut auth = long_auth();
        let request = AuthRequest::new_long_term("manato", "player", "pass", 10);
        assert!(auth.validate(&request));
        assert!(!auth.validate(&request), "replay rejected");
        let wrong_password = AuthRequest::new_long_term("manato", "player", "bad", 11);
        assert!(!auth.validate(&wrong_password));
        let wrong_realm = AuthRequest::new_long_term("other", "player", "pass", 12);
        assert!(!auth.validate(&wrong_realm));
        let unknown = AuthRequest::new_long_term("manato", "nobody", "pass", 13);
        assert!(!auth.validate(&unknown));
        let shared = AuthRequest::new("pass", "player", 14);
        assert!(!auth.validate(&shared), "wrong credential mode rejected");
    }

    #[test]
    fn rejects_bad_username_and_malformed_frames() {
        let mut auth = RelayAuthenticator::new("right");
        assert!(!auth.validate(&AuthRequest::new("wrong", "player", 1)));
        assert!(!auth.validate(&AuthRequest::new("right", "", 2)));
        let long = "x".repeat(33);
        assert!(!auth.validate(&AuthRequest::new("right", &long, 3)));
        assert!(AuthRequest::decode(b"not a join").is_none());
        assert!(AuthRequest::decode(b"MANATO_RELAY_AUTH_JOIN_V1:garbage").is_none());
    }

    #[test]
    fn decode_rejects_oversized_length_prefix() {
        let mut bytes = AUTH_JOIN.to_vec();
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(AuthRequest::decode(&bytes).is_none());
    }
}
