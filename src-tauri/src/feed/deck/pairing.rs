//! Pairing: a six-digit code shown in Settings, swapped once for a random
//! token. Only the token's SHA-256 is kept, in the keychain. Five wrong codes
//! in a row make a new code; pairing again replaces the old token.

use std::sync::Arc;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;

/// Keychain account for the paired token's hash.
pub const TOKEN_ACCOUNT: &str = "streamdeck-token-hash";
/// Wrong codes before the code changes.
pub const MAX_WRONG: u32 = 5;

/// Why pairing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    WrongCode,
    Keychain(String),
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Compare without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn new_code() -> String {
    format!("{:06}", rand::random::<u32>() % 1_000_000)
}

struct State {
    code: String,
    wrong: u32,
    /// The stored hash, once read (None inside: nothing paired).
    hash: Option<Option<String>>,
}

pub struct Pairing {
    store: Arc<dyn TokenStore>,
    state: Mutex<State>,
}

impl Pairing {
    pub fn new(store: Arc<dyn TokenStore>) -> Self {
        Self { store, state: Mutex::new(State { code: new_code(), wrong: 0, hash: None }) }
    }

    /// The code to type into the Stream Deck.
    pub fn code(&self) -> String {
        self.state.lock().code.clone()
    }

    /// Show a new code (the old one stops working).
    pub fn rotate(&self) {
        let mut s = self.state.lock();
        s.code = new_code();
        s.wrong = 0;
    }

    fn stored(&self, s: &mut State) -> Option<String> {
        if s.hash.is_none() {
            let read = match self.store.get(TOKEN_ACCOUNT) {
                Ok(t) => t.map(|t| t.expose().to_owned()),
                Err(e) => {
                    // Not cached, so it's read again next time.
                    tracing::debug!("stream deck pairing not read: {e}");
                    return None;
                }
            };
            s.hash = Some(read);
        }
        s.hash.clone().flatten()
    }

    /// A Stream Deck is paired.
    pub fn paired(&self) -> bool {
        let mut s = self.state.lock();
        self.stored(&mut s).is_some()
    }

    /// Whether `token` is the paired one.
    pub fn check(&self, token: &str) -> bool {
        if token.is_empty() || token.len() > 256 {
            return false;
        }
        let mut s = self.state.lock();
        self.stored(&mut s).is_some_and(|h| same(&h, &hash(token)))
    }

    /// Swap the code for a new token. The code changes after a success, and
    /// after [`MAX_WRONG`] wrong ones.
    pub fn pair(&self, code: &str) -> Result<String, PairError> {
        let mut s = self.state.lock();
        if !same(code.trim(), &s.code) {
            s.wrong += 1;
            if s.wrong >= MAX_WRONG {
                s.code = new_code();
                s.wrong = 0;
            }
            return Err(PairError::WrongCode);
        }
        let token = hex(&rand::random::<[u8; 32]>());
        let h = hash(&token);
        let stored = Token::new(h.clone()).map_err(|e| PairError::Keychain(e.to_string()))?;
        self.store.set(TOKEN_ACCOUNT, &stored).map_err(|e| PairError::Keychain(e.to_string()))?;
        s.hash = Some(Some(h));
        s.code = new_code();
        s.wrong = 0;
        Ok(token)
    }

    /// Forget the paired Stream Deck and show a new code.
    pub fn revoke(&self) -> AppResult<()> {
        let mut s = self.state.lock();
        self.store.delete(TOKEN_ACCOUNT).map_err(|e| AppError::Internal(format!("couldn't forget the Stream Deck: {e}")))?;
        s.hash = Some(None);
        s.code = new_code();
        s.wrong = 0;
        Ok(())
    }
}
