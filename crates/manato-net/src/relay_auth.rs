//! TURN-style shared-secret authentication for relays.
//!
//! A client submits a username, nonce, and HMAC-SHA256 over
//! `username || 0 || nonce`. The relay validates the MAC and rejects replayed
//! nonces per username.

use std::collections::HashSet;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const AUTH_JOIN: &[u8] = b"MANATO_RELAY_AUTH_JOIN_V1:";
const MAX_USERNAME: usize = 32;
const MAX_SEEN: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub nonce: u64,
    pub mac: [u8; 32],
}

impl AuthRequest {
    pub fn new(secret: &str, username: &str, nonce: u64) -> Self {
        Self {
            username: username.to_string(),
            nonce,
            mac: compute_mac(secret, username, nonce),
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
        if body.len() > 256 {
            return None;
        }
        bincode::deserialize(body).ok()
    }
}

fn compute_mac(secret: &str, username: &str, nonce: u64) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key");
    mac.update(username.as_bytes());
    mac.update(&[0]);
    mac.update(&nonce.to_be_bytes());
    mac.finalize().into_bytes().into()
}

/// Relay-side validator with replay protection.
pub struct RelayAuthenticator {
    secret: String,
    seen: HashSet<(String, u64)>,
}

impl RelayAuthenticator {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            seen: HashSet::new(),
        }
    }

    pub fn validate(&mut self, request: &AuthRequest) -> bool {
        let username = request.username.trim();
        if username.is_empty()
            || username.chars().count() > MAX_USERNAME
            || username != request.username
        {
            return false;
        }
        let expected = compute_mac(&self.secret, username, request.nonce);
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes()).expect("hmac key");
        mac.update(username.as_bytes());
        mac.update(&[0]);
        mac.update(&request.nonce.to_be_bytes());
        if mac.verify_slice(&request.mac).is_err() || expected != request.mac {
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

    #[test]
    fn validates_credentials_and_rejects_replay() {
        let mut auth = RelayAuthenticator::new("s3cret");
        let request = AuthRequest::new("s3cret", "player", 7);
        let encoded = request.encode();
        let decoded = AuthRequest::decode(&encoded).expect("decode");
        assert!(auth.validate(&decoded));
        assert!(!auth.validate(&decoded), "nonce replay rejected");
        let next = AuthRequest::new("s3cret", "player", 8);
        assert!(auth.validate(&next));
    }

    #[test]
    fn rejects_bad_secret_username_and_malformed_frames() {
        let mut auth = RelayAuthenticator::new("right");
        assert!(!auth.validate(&AuthRequest::new("wrong", "player", 1)));
        assert!(!auth.validate(&AuthRequest::new("right", "", 2)));
        let long = "x".repeat(33);
        assert!(!auth.validate(&AuthRequest::new("right", &long, 3)));
        assert!(AuthRequest::decode(b"not a join").is_none());
        assert!(AuthRequest::decode(b"MANATO_RELAY_AUTH_JOIN_V1:garbage").is_none());
    }
}
