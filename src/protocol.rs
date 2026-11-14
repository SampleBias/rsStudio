//! The local, versioned control transport. See docs/protocol.md.
use std::io::{self, Read, Write};

pub const MAX_FRAME: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: String,
    pub id: u64,
    pub payload: String,
}

pub fn write_message(w: &mut impl Write, kind: &str, id: u64, payload: &str) -> io::Result<()> {
    if kind.is_empty() || !kind.bytes().all(|c| c.is_ascii_uppercase()) || payload.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid message",
        ));
    }
    let bytes = format!("{kind}\t{id}\t{payload}").into_bytes();
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message exceeds 1 MiB",
        ));
    }
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(&bytes)?;
    w.flush()
}

pub fn read_message(r: &mut impl Read) -> io::Result<Message> {
    let mut header = [0; 4];
    r.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-8"))?;
    let mut fields = text.splitn(3, '\t');
    let kind = fields.next().unwrap_or_default();
    let id = fields.next().and_then(|s| s.parse().ok());
    let payload = fields.next();
    if kind.is_empty()
        || !kind.bytes().all(|c| c.is_ascii_uppercase())
        || id.is_none()
        || payload.is_none()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid message fields",
        ));
    }
    Ok(Message {
        kind: kind.to_owned(),
        id: id.unwrap(),
        payload: payload.unwrap().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_unicode_tabs_and_lines() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, "EVAL", 7, "π <- '\t>'\nπ").unwrap();
        let message = read_message(&mut &bytes[..]).unwrap();
        assert_eq!(
            message,
            Message {
                kind: "EVAL".into(),
                id: 7,
                payload: "π <- '\t>'\nπ".into()
            }
        );
    }
    #[test]
    fn rejects_truncated_oversized_and_invalid_frames() {
        assert!(read_message(&mut &[0, 0, 0, 5, b'a'][..]).is_err());
        assert!(read_message(&mut &((MAX_FRAME as u32 + 1).to_be_bytes())[..]).is_err());
        assert!(read_message(&mut &[0, 0, 0, 1, 255][..]).is_err());
        assert!(write_message(&mut Vec::new(), "EVAL", 1, &"x".repeat(MAX_FRAME)).is_err());
    }
}
