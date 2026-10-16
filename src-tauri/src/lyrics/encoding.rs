//! Lyrics files as text. Sidecar .lrc and .txt files are often not UTF-8
//! (Shift-JIS from Japanese tools, Windows-1252 from older Western ones):
//! a byte order mark decides first, then valid UTF-8, then a guess from the
//! bytes, instead of turning every other character into U+FFFD.

use std::borrow::Cow;

use encoding_rs::{Encoding, UTF_8};

/// The text of a lyrics file in whatever encoding it was saved in.
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if let Some((enc, bom)) = Encoding::for_bom(bytes) {
        return enc.decode_without_bom_handling(&bytes[bom..]).0;
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(s);
    }
    let mut det = chardetng::EncodingDetector::new();
    det.feed(bytes, true);
    let enc = det.guess(None, true);
    let enc = if enc == UTF_8 { encoding_rs::WINDOWS_1252 } else { enc };
    enc.decode_without_bom_handling(bytes).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_byte_order_marks() {
        assert_eq!(decode("[00:01.00]♪ café".as_bytes()), "[00:01.00]♪ café");
        assert_eq!(decode(b"\xEF\xBB\xBFhello"), "hello");
        // UTF-16, little and big endian.
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("héllo".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("héllo".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        assert_eq!(decode(&le), "héllo");
        assert_eq!(decode(&be), "héllo");
    }

    #[test]
    fn shift_jis_lrc() {
        let text = "[00:12.34]君の名前を呼んでいる\n[00:18.00]夜明けまで歌おう\n";
        let (bytes, _, bad) = encoding_rs::SHIFT_JIS.encode(text);
        assert!(!bad);
        assert_eq!(decode(&bytes), text);
    }

    #[test]
    fn windows_1252_lrc() {
        let text = "[00:05.00]Déjà vu, señorita, ça va\n[00:09.00]Naïve café “quoted”\n";
        let (bytes, _, bad) = encoding_rs::WINDOWS_1252.encode(text);
        assert!(!bad);
        assert!(std::str::from_utf8(&bytes).is_err());
        assert_eq!(decode(&bytes), text);
    }
}
