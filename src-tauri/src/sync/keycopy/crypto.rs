//! The cryptography of a key copy.
//!
//! SPAKE2 (Ed25519 group) over the 6-digit code turns the code into a
//! strong shared key; someone listening learns nothing they could guess the
//! code from, and a device pretending gets one guess per exchange. The
//! sender is side A, the receiver side B, each named by its device id.
//! From the shared key come a confirmation value (the receiver proves it
//! has the same key before anything is sent) and the key that seals the
//! keys with ChaCha20-Poly1305, both devices' ids bound in as associated
//! data.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::error::{AppError, AppResult};
use crate::remote::peer::auth::hmac_sha256;

/// The sender's state between its two messages.
pub struct SenderState(Spake2<Ed25519Group>);

/// A shared key (never printed).
#[derive(Clone)]
pub struct SharedKey([u8; 32]);

impl std::fmt::Debug for SharedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedKey(..)")
    }
}

impl SharedKey {
    fn sub(&self, label: &[u8]) -> [u8; 32] {
        hmac_sha256(&self.0, label)
    }

    /// A key for another use of the same exchange (device pairing).
    pub fn derive(&self, label: &str) -> [u8; 32] {
        self.sub(label.as_bytes())
    }

    /// What the receiver sends to show it has the same key.
    pub fn confirmation(&self) -> [u8; 32] {
        self.sub(b"platter-keys-v1 confirm")
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        let k: Key = self.sub(b"platter-keys-v1 seal").into();
        ChaCha20Poly1305::new(&k)
    }
}

fn ids(sender: &str, receiver: &str) -> (Identity, Identity) {
    (Identity::new(sender.as_bytes()), Identity::new(receiver.as_bytes()))
}

fn aad(sender: &str, receiver: &str) -> Vec<u8> {
    format!("platter-keys-v1\n{sender}\n{receiver}").into_bytes()
}

/// The sender's first message.
pub fn sender_start(code: &str, sender: &str, receiver: &str) -> (SenderState, Vec<u8>) {
    let (a, b) = ids(sender, receiver);
    let (s, msg) = Spake2::<Ed25519Group>::start_a(&Password::new(code.as_bytes()), &a, &b);
    (SenderState(s), msg)
}

/// The receiver's answer to the sender's message, and the key it reached.
pub fn receiver_answer(code: &str, sender: &str, receiver: &str, msg_a: &[u8]) -> AppResult<(Vec<u8>, SharedKey)> {
    let (a, b) = ids(sender, receiver);
    let (s, msg_b) = Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &a, &b);
    let key = s.finish(msg_a).map_err(|_| AppError::Invalid("Bad key exchange message.".into()))?;
    Ok((msg_b, SharedKey(to32(&key)?)))
}

/// The key the sender reached from the receiver's answer.
pub fn sender_finish(state: SenderState, msg_b: &[u8]) -> AppResult<SharedKey> {
    let key = state.0.finish(msg_b).map_err(|_| AppError::Invalid("Bad key exchange message.".into()))?;
    Ok(SharedKey(to32(&key)?))
}

fn to32(v: &[u8]) -> AppResult<[u8; 32]> {
    v.try_into().map_err(|_| AppError::Internal("unexpected key length".into()))
}

/// Seal: (nonce, ciphertext).
pub fn seal(key: &SharedKey, sender: &str, receiver: &str, plain: &[u8]) -> AppResult<([u8; 12], Vec<u8>)> {
    let nonce: [u8; 12] = rand::random();
    let aad = aad(sender, receiver);
    let ct = key
        .cipher()
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: &aad })
        .map_err(|_| AppError::Internal("couldn't seal".into()))?;
    Ok((nonce, ct))
}

/// Open what [`seal`] made; fails if the key, the devices or a byte differ.
pub fn open(key: &SharedKey, sender: &str, receiver: &str, nonce: &[u8], sealed: &[u8]) -> AppResult<Vec<u8>> {
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| AppError::Invalid("Bad nonce.".into()))?;
    let aad = aad(sender, receiver);
    key.cipher()
        .decrypt(&Nonce::from(nonce), Payload { msg: sealed, aad: &aad })
        .map_err(|_| AppError::Invalid("The keys couldn't be opened.".into()))
}

pub fn hex(b: &[u8]) -> String {
    crate::remote::peer::auth::hex(b)
}

pub fn unhex(s: &str) -> AppResult<Vec<u8>> {
    if !s.is_ascii() || !s.len().is_multiple_of(2) || s.len() > 64 * 1024 {
        return Err(AppError::Invalid("Bad hex.".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| AppError::Invalid("Bad hex.".into())))
        .collect()
}

/// Constant-time comparison.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
