//! Length-prefixed JSON frames, each with room for bytes beside it.
//!
//! One frame = a little-endian `u32` length and that many bytes of JSON, then
//! a little-endian `u64` length and that many raw bytes. JSON rather than a
//! binary codec on purpose: frames are debuggable with a hex dump and `jq`,
//! and the daemon stores op frames verbatim in its log, so the log stays
//! greppable. A document's archive travels in the raw half, where it costs
//! its own size rather than four times it, and is not bounded by what a
//! `u32` can count. Both lengths are capped so a corrupted or hostile peer
//! cannot make us allocate the moon.

use std::io::{Read, Write};

use core_document::server::Payload;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Largest accepted JSON header. Headers describe work; they never carry it.
pub const MAX_FRAME_BYTES: u32 = 1 << 30;

/// Largest accepted payload. A document with a big assembly in it runs to
/// gigabytes, so this is generous; anything larger is a protocol error, not
/// a bigger buffer.
pub const MAX_PAYLOAD_BYTES: u64 = 64 << 30;

pub fn write_frame<W: Write, T: Serialize + Payload>(
    mut out: W,
    message: &T,
) -> std::io::Result<()> {
    let payload = message.payload();
    let header = serde_json::to_vec(message)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len = u32::try_from(header.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"))?;
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    if payload.len() as u64 > MAX_PAYLOAD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "payload too large",
        ));
    }
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&header)?;
    out.write_all(&(payload.len() as u64).to_le_bytes())?;
    out.write_all(payload)?;
    out.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned + Payload>(mut input: R) -> std::io::Result<T> {
    let mut len_bytes = [0u8; 4];
    input.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes);
    if len > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame length exceeds cap",
        ));
    }
    let mut header = vec![0u8; len as usize];
    input.read_exact(&mut header)?;
    let mut message: T = serde_json::from_slice(&header)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let mut payload_len = [0u8; 8];
    input.read_exact(&mut payload_len)?;
    let payload_len = u64::from_le_bytes(payload_len);
    if payload_len > MAX_PAYLOAD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "payload length exceeds cap",
        ));
    }
    if payload_len > 0 {
        let mut payload = vec![0u8; payload_len as usize];
        input.read_exact(&mut payload)?;
        message.set_payload(payload);
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::server::{ClientMessage, ServerMessage};

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

    /// A payload larger than a `u32` could describe as JSON numbers, which is
    /// what a document with a big assembly in it looks like.
    #[test]
    fn a_large_payload_survives_a_round_trip() {
        let bytes: Vec<u8> = (0..(8 << 20)).map(|i| (i % 251) as u8).collect();
        let message = ClientMessage::SaveDocument {
            path: std::path::PathBuf::from("/tmp/big.prtcad"),
            bytes: bytes.clone(),
            at_seq: 7,
        };

        let mut wire = Vec::new();
        write_frame(&mut wire, &message).expect("write");
        // The archive crosses as itself: the frame is the header plus the
        // bytes, not four times them.
        assert!(wire.len() < bytes.len() + 4096, "wire grew: {}", wire.len());

        let back: ClientMessage = read_frame(&wire[..]).expect("read");
        match back {
            ClientMessage::SaveDocument {
                bytes: got,
                at_seq,
                path,
            } => {
                assert_eq!(got, bytes);
                assert_eq!(at_seq, 7);
                assert_eq!(path, std::path::PathBuf::from("/tmp/big.prtcad"));
            }
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn a_message_without_bytes_carries_none() {
        let message = ServerMessage::SaveFailed {
            path: std::path::PathBuf::from("/tmp/x"),
            error: "no".into(),
        };
        let mut wire = Vec::new();
        write_frame(&mut wire, &message).expect("write");
        let back: ServerMessage = read_frame(&wire[..]).expect("read");
        assert!(matches!(back, ServerMessage::SaveFailed { .. }));
    }
}
