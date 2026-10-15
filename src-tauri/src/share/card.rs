//! Share cards (docs/IPC.md "Share cards"). The UI draws the card on a
//! canvas; the core gives it the QR code for the item's Platter link, and
//! puts the finished image on the clipboard or in a file the user picks.
//! Nothing is uploaded.
//!
//! The QR code only ever holds the `platter://` link from [`ShareService::info`]
//! (the item, an optional start time and the 8-hex server tag), never an
//! address, the server's id or a token.

use crate::error::{AppError, AppResult};
use crate::sync::keycopy::{Qr, qr_text};

use super::{ShareKind, ShareService};

/// Largest card side accepted (the UI makes 1080 by 1920 at most).
pub const MAX_SIDE: u32 = 4096;
/// Largest PNG accepted for saving.
pub const MAX_PNG_BYTES: usize = 32 * 1024 * 1024;
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The QR code on a card and the text it holds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardQr {
    pub size: usize,
    pub rows: Vec<String>,
    /// What scanning it gives: the Platter link.
    pub text: String,
}

impl ShareService {
    /// The QR code for an item's Platter link (with the start time for a song).
    pub async fn card_qr(&self, kind: ShareKind, id: &str, at_seconds: Option<u32>) -> AppResult<CardQr> {
        let info = self.info(kind, id, at_seconds).await?;
        let Qr { size, rows } = qr_text(&info.app_link).ok_or_else(|| AppError::Internal("QR code".into()))?;
        Ok(CardQr { size, rows, text: info.app_link })
    }
}

/// Raw RGBA pixels of the right length for their size.
pub fn check_rgba(len: usize, width: u32, height: u32) -> AppResult<()> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(AppError::Invalid("That card is the wrong size.".into()));
    }
    if len != width as usize * height as usize * 4 {
        return Err(AppError::Invalid("That card image is incomplete.".into()));
    }
    Ok(())
}

/// A PNG file, not too big.
pub fn check_png(bytes: &[u8]) -> AppResult<()> {
    if bytes.len() > MAX_PNG_BYTES || !bytes.starts_with(&PNG_MAGIC) {
        return Err(AppError::Invalid("That card image isn't a PNG.".into()));
    }
    Ok(())
}

/// A file name for the card: the title without characters file systems
/// refuse, at most 80 characters, ending in `.png`.
pub fn file_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.trim_matches('.').chars().take(80).collect();
    let base = if clean.trim().is_empty() { "Platter card".to_string() } else { clean.trim().to_string() };
    format!("{base}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_must_match_its_size() {
        assert!(check_rgba(4 * 10 * 20, 10, 20).is_ok());
        assert!(check_rgba(4 * 10 * 20 - 1, 10, 20).is_err());
        assert!(check_rgba(0, 0, 0).is_err());
        assert!(check_rgba(4, MAX_SIDE + 1, 1).is_err());
    }

    #[test]
    fn only_pngs_are_saved() {
        let mut png = PNG_MAGIC.to_vec();
        png.extend([0; 16]);
        assert!(check_png(&png).is_ok());
        assert!(check_png(b"GIF89a....").is_err());
        assert!(check_png(&[]).is_err());
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("Song: A/B?"), "Song A B.png");
        assert_eq!(file_name("   "), "Platter card.png");
        assert_eq!(file_name("..."), "Platter card.png");
        assert_eq!(file_name(&"x".repeat(200)).len(), 84);
    }
}
