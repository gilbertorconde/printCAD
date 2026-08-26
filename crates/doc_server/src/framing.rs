//! Length-prefixed JSON frames.
//!
//! One frame = a little-endian `u32` byte length, then that many bytes of
//! JSON. JSON rather than a binary codec on purpose: frames are debuggable
//! with a hex dump and `jq`, and the daemon stores op frames verbatim in its
//! log, so the log stays greppable. The length prefix is capped so a
//! corrupted or hostile peer cannot make us allocate the moon.

use std::io::{Read, Write};

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Largest accepted frame. Imports carry whole STEP files, so this is
/// generous; anything larger is a protocol error, not a bigger buffer.
pub const MAX_FRAME_BYTES: u32 = 1 << 30;

pub fn write_frame<W: Write, T: Serialize>(mut out: W, message: &T) -> std::io::Result<()> {
    let payload = serde_json::to_vec(message)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"))?;
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&payload)?;
    out.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned>(mut input: R) -> std::io::Result<T> {
    let mut len_bytes = [0u8; 4];
    input.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame length exceeds cap",
        ));
    }
    let mut payload = vec![0u8; len as usize];
    input.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::server::ClientMessage;

    #[test]
    fn a_frame_round_trips() {
        let mut wire = Vec::new();
        write_frame(
            &mut wire,
            &ClientMessage::Hello {
                protocol: 7,
                actor: uuid::Uuid::nil(),
            },
        )
        .expect("write");
        let back: ClientMessage = read_frame(wire.as_slice()).expect("read");
        match back {
            ClientMessage::Hello { protocol, .. } => assert_eq!(protocol, 7),
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn an_oversized_length_is_refused_without_allocating() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&u32::MAX.to_le_bytes());
        let result: std::io::Result<ClientMessage> = read_frame(wire.as_slice());
        assert!(result.is_err());
    }
}
