//! Proof Key for Code Exchange (RFC 7636): the sign-in needs no client
//! secret. The verifier stays in memory for one sign-in; only its hash goes
//! out with the sign-in link.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// A fresh verifier: 32 random bytes as base64url, 43 characters from the
/// unreserved set the RFC allows.
pub fn verifier() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

/// The S256 challenge for a verifier.
pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A random value tying the sign-in answer to this sign-in.
pub fn state() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 24]>())
}
