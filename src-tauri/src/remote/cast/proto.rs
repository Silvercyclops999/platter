//! Cast v2 message framing: each message is a 4-byte big-endian length and
//! a protobuf `CastMessage`. Only the seven fields of that one message are
//! needed, so they're encoded and decoded by hand.
//!
//! ```text
//! message CastMessage {
//!   required ProtocolVersion protocol_version = 1;  // CASTV2_1_0 = 0
//!   required string source_id = 2;
//!   required string destination_id = 3;
//!   required string namespace = 4;
//!   required PayloadType payload_type = 5;          // STRING = 0, BINARY = 1
//!   optional string payload_utf8 = 6;
//!   optional bytes payload_binary = 7;
//! }
//! ```

/// Largest frame accepted (the protocol's own limit is 64 KiB).
pub const MAX_FRAME: usize = 64 * 1024;

pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
pub const NS_PLEX: &str = "urn:x-cast:plex";

pub const SENDER: &str = "sender-0";
pub const RECEIVER: &str = "receiver-0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastMessage {
    pub source: String,
    pub destination: String,
    pub namespace: String,
    /// Text payload (JSON for every namespace used here).
    pub payload: String,
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn bytes_field(out: &mut Vec<u8>, field: u8, b: &[u8]) {
    out.push((field << 3) | 2);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

impl CastMessage {
    pub fn new(source: &str, destination: &str, namespace: &str, payload: String) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            namespace: namespace.into(),
            payload,
        }
    }

    /// The protobuf body (no length prefix).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.payload.len() + 96);
        out.extend_from_slice(&[0x08, 0x00]); // protocol_version = CASTV2_1_0
        bytes_field(&mut out, 2, self.source.as_bytes());
        bytes_field(&mut out, 3, self.destination.as_bytes());
        bytes_field(&mut out, 4, self.namespace.as_bytes());
        out.extend_from_slice(&[0x28, 0x00]); // payload_type = STRING
        bytes_field(&mut out, 6, self.payload.as_bytes());
        out
    }

    /// Length-prefixed frame, ready to write.
    pub fn frame(&self) -> Vec<u8> {
        let body = self.encode();
        let mut out = Vec::with_capacity(body.len() + 4);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Parses a protobuf body. Unknown fields are skipped; a binary payload
    /// is kept as lossy text (none of the namespaces used here send one).
    pub fn decode(buf: &[u8]) -> Option<Self> {
        let mut m = CastMessage::new("", "", "", String::new());
        let mut i = 0usize;
        let read_varint = |i: &mut usize| -> Option<u64> {
            let mut v = 0u64;
            for shift in (0..64).step_by(7) {
                let b = *buf.get(*i)?;
                *i += 1;
                v |= u64::from(b & 0x7f) << shift;
                if b & 0x80 == 0 {
                    return Some(v);
                }
            }
            None
        };
        while i < buf.len() {
            let key = read_varint(&mut i)?;
            let (field, wire) = (key >> 3, key & 7);
            match wire {
                0 => {
                    read_varint(&mut i)?;
                }
                1 => i = i.checked_add(8)?,
                5 => i = i.checked_add(4)?,
                2 => {
                    let len = usize::try_from(read_varint(&mut i)?).ok()?;
                    let end = i.checked_add(len)?;
                    let b = buf.get(i..end)?;
                    i = end;
                    let s = || String::from_utf8_lossy(b).into_owned();
                    match field {
                        2 => m.source = s(),
                        3 => m.destination = s(),
                        4 => m.namespace = s(),
                        6 | 7 => m.payload = s(),
                        _ => {}
                    }
                }
                _ => return None,
            }
        }
        if i != buf.len() {
            return None;
        }
        Some(m)
    }
}

/// Reads one frame from an async stream. None at a clean end of stream.
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<CastMessage>> {
    use tokio::io::AsyncReadExt;
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "cast frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    CastMessage::decode(&body)
        .map(Some)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad cast message"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let m = CastMessage::new(SENDER, RECEIVER, NS_RECEIVER, r#"{"type":"GET_STATUS","requestId":1}"#.into());
        let body = m.encode();
        assert_eq!(&body[..4], &[0x08, 0x00, 0x12, 0x08]);
        assert_eq!(CastMessage::decode(&body).unwrap(), m);
        let f = m.frame();
        assert_eq!(u32::from_be_bytes(f[..4].try_into().unwrap()) as usize, body.len());
    }

    #[test]
    fn long_payloads_use_multi_byte_lengths() {
        let m = CastMessage::new("a", "b", NS_MEDIA, "x".repeat(300));
        let d = CastMessage::decode(&m.encode()).unwrap();
        assert_eq!(d.payload.len(), 300);
    }

    #[test]
    fn unknown_fields_are_skipped_and_junk_is_refused() {
        let mut body = CastMessage::new("a", "b", NS_MEDIA, "{}".into()).encode();
        body.extend_from_slice(&[0x40, 0x05]); // field 8, varint 5
        assert_eq!(CastMessage::decode(&body).unwrap().payload, "{}");
        assert!(CastMessage::decode(&[0x12, 0x09, b'a']).is_none());
        assert!(CastMessage::decode(&[0x0b]).is_none());
    }

    #[tokio::test]
    async fn frames_read_from_a_stream() {
        let m = CastMessage::new("a", "b", NS_HEARTBEAT, r#"{"type":"PING"}"#.into());
        let mut bytes = m.frame();
        bytes.extend_from_slice(&m.frame());
        let mut r = &bytes[..];
        assert_eq!(read_frame(&mut r).await.unwrap().unwrap(), m);
        assert_eq!(read_frame(&mut r).await.unwrap().unwrap(), m);
        assert!(read_frame(&mut r).await.unwrap().is_none());
        let huge = ((MAX_FRAME + 1) as u32).to_be_bytes();
        assert!(read_frame(&mut &huge[..]).await.is_err());
    }
}
