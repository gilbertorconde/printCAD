//! Request identifiers and the request/response packet layout.
//!
//! A request reuses the 32-byte packet: word 0 is [`REQ_TAG`] combined with
//! the request id, words 1..=6 are its arguments and word 7 is the status the
//! daemon writes back (negative means it refused). The daemon echoes the
//! request id, so a reply can always be matched to what asked for it.
//!
//! Strings do not fit in one packet, so they travel 24 bytes at a time with a
//! remaining-length field in word 7 — its low 16 bits are the bytes still to
//! come, including the current chunk, and bit 16 marks every packet after the
//! first.

use crate::Error;
use crate::codec::{PACKET_BYTES, Words, bytes_from_words, words_from_bytes};

pub(crate) const REQ_TAG: i32 = 0x7faa_0000;

const REQ_BASE: i32 = 0x1000;
pub(crate) const REQ_SET_NAME: i32 = REQ_BASE;
pub(crate) const REQ_SET_SENS: i32 = REQ_BASE + 1;
pub(crate) const REQ_GET_SENS: i32 = REQ_BASE + 2;
pub(crate) const REQ_SET_EVMASK: i32 = REQ_BASE + 3;
pub(crate) const REQ_GET_EVMASK: i32 = REQ_BASE + 4;

const REQ_DEV_BASE: i32 = 0x2000;
pub(crate) const REQ_DEV_NAME: i32 = REQ_DEV_BASE;
pub(crate) const REQ_DEV_PATH: i32 = REQ_DEV_BASE + 1;
pub(crate) const REQ_DEV_NAXES: i32 = REQ_DEV_BASE + 2;
pub(crate) const REQ_DEV_NBUTTONS: i32 = REQ_DEV_BASE + 3;
pub(crate) const REQ_DEV_USBID: i32 = REQ_DEV_BASE + 4;
pub(crate) const REQ_DEV_TYPE: i32 = REQ_DEV_BASE + 5;

/// Settings that belong to the daemon rather than to one client. Every pair
/// is set-then-get, in the order the daemon lists them.
const REQ_CFG_BASE: i32 = 0x3000;
pub(crate) const REQ_SCFG_SENS: i32 = REQ_CFG_BASE;
pub(crate) const REQ_GCFG_SENS: i32 = REQ_CFG_BASE + 1;
pub(crate) const REQ_SCFG_SENS_AXIS: i32 = REQ_CFG_BASE + 2;
pub(crate) const REQ_GCFG_SENS_AXIS: i32 = REQ_CFG_BASE + 3;
pub(crate) const REQ_SCFG_DEADZONE: i32 = REQ_CFG_BASE + 4;
pub(crate) const REQ_GCFG_DEADZONE: i32 = REQ_CFG_BASE + 5;
pub(crate) const REQ_SCFG_INVERT: i32 = REQ_CFG_BASE + 6;
pub(crate) const REQ_GCFG_INVERT: i32 = REQ_CFG_BASE + 7;
pub(crate) const REQ_SCFG_AXISMAP: i32 = REQ_CFG_BASE + 8;
pub(crate) const REQ_GCFG_AXISMAP: i32 = REQ_CFG_BASE + 9;
pub(crate) const REQ_SCFG_BNMAP: i32 = REQ_CFG_BASE + 10;
pub(crate) const REQ_GCFG_BNMAP: i32 = REQ_CFG_BASE + 11;
pub(crate) const REQ_SCFG_BNACTION: i32 = REQ_CFG_BASE + 12;
pub(crate) const REQ_GCFG_BNACTION: i32 = REQ_CFG_BASE + 13;
pub(crate) const REQ_SCFG_KBMAP: i32 = REQ_CFG_BASE + 14;
pub(crate) const REQ_GCFG_KBMAP: i32 = REQ_CFG_BASE + 15;
pub(crate) const REQ_SCFG_SWAPYZ: i32 = REQ_CFG_BASE + 16;
pub(crate) const REQ_GCFG_SWAPYZ: i32 = REQ_CFG_BASE + 17;
pub(crate) const REQ_SCFG_LED: i32 = REQ_CFG_BASE + 18;
pub(crate) const REQ_GCFG_LED: i32 = REQ_CFG_BASE + 19;
pub(crate) const REQ_SCFG_GRAB: i32 = REQ_CFG_BASE + 20;
pub(crate) const REQ_GCFG_GRAB: i32 = REQ_CFG_BASE + 21;
pub(crate) const REQ_SCFG_SERDEV: i32 = REQ_CFG_BASE + 22;
pub(crate) const REQ_GCFG_SERDEV: i32 = REQ_CFG_BASE + 23;
pub(crate) const REQ_SCFG_REPEAT: i32 = REQ_CFG_BASE + 24;
pub(crate) const REQ_GCFG_REPEAT: i32 = REQ_CFG_BASE + 25;
pub(crate) const REQ_SCFG_SOCKET: i32 = REQ_CFG_BASE + 26;
pub(crate) const REQ_GCFG_SOCKET: i32 = REQ_CFG_BASE + 27;
pub(crate) const REQ_CFG_SAVE: i32 = 0x3ffe;
pub(crate) const REQ_CFG_RESTORE: i32 = 0x3fff;
pub(crate) const REQ_CFG_RESET: i32 = 0x4000;

pub(crate) const REQ_CHANGE_PROTO: i32 = 0x5500;

/// The newest protocol version this client speaks.
pub(crate) const MAX_PROTO_VER: i32 = 1;

/// Bytes of string payload per packet.
const CHUNK: usize = 24;
/// Set in the remaining-length word of every packet after the first.
const CONT_BIT: i32 = 0x1_0000;
/// The remaining-length field has 16 bits, so longer strings are cut short.
const MAX_STRING: usize = 0xffff;

/// Builds a request packet carrying up to six argument words.
pub(crate) fn request_packet(request: i32, args: &[i32]) -> Words {
    let mut words = [0i32; 8];
    words[0] = REQ_TAG | request;
    for (slot, arg) in words[1..7].iter_mut().zip(args) {
        *slot = *arg;
    }
    words
}

/// Checks that a reply answers `request` rather than something else.
pub(crate) fn check_reply(request: i32, words: &Words) -> Result<(), Error> {
    if words[0] != (REQ_TAG | request) {
        return Err(Error::Protocol("a reply answered a different request"));
    }
    Ok(())
}

/// Checks that a reply answers `request` and that the daemon accepted it.
pub(crate) fn reply_status(request: i32, words: &Words) -> Result<(), Error> {
    check_reply(request, words)?;
    if words[7] < 0 {
        return Err(Error::Refused);
    }
    Ok(())
}

/// The packets that carry `text` as the payload of `request`.
///
/// An empty string still takes one packet, which is how the daemon learns the
/// transfer is over.
pub(crate) fn string_packets(request: i32, text: &str) -> Vec<Words> {
    let bytes = &text.as_bytes()[..text.len().min(MAX_STRING)];
    let mut packets = Vec::new();
    let mut sent = 0;
    loop {
        let remaining = bytes.len() - sent;
        let take = remaining.min(CHUNK);

        let mut raw = [0u8; PACKET_BYTES];
        raw[4..4 + take].copy_from_slice(&bytes[sent..sent + take]);
        let mut words = words_from_bytes(&raw);
        words[0] = REQ_TAG | request;
        words[7] = remaining as i32 | if sent == 0 { 0 } else { CONT_BIT };
        packets.push(words);

        sent += take;
        if sent >= bytes.len() {
            return packets;
        }
    }
}

/// Reassembles a string that arrives one chunk per packet.
#[derive(Default)]
pub(crate) struct StringReader {
    text: Vec<u8>,
    expect: usize,
    started: bool,
}

impl StringReader {
    /// Takes one packet; returns the string once the last chunk has landed.
    pub(crate) fn push(&mut self, words: &Words) -> Result<Option<String>, Error> {
        if words[7] < 0 {
            return Err(Error::Refused);
        }
        let remaining = (words[7] & 0xffff) as usize;
        if words[7] & CONT_BIT == 0 {
            self.text.clear();
            self.expect = remaining;
            self.started = true;
        } else if !self.started {
            return Err(Error::Protocol("a string continued before it began"));
        }
        if remaining != self.expect {
            return Err(Error::Protocol("a string chunk arrived out of order"));
        }

        let take = remaining.min(CHUNK);
        let bytes = bytes_from_words(words);
        self.text.extend_from_slice(&bytes[4..4 + take]);
        self.expect -= take;

        if self.expect == 0 {
            self.started = false;
            let text = String::from_utf8_lossy(&self.text).into_owned();
            self.text.clear();
            return Ok(Some(text));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) -> String {
        let mut reader = StringReader::default();
        let mut result = None;
        for packet in string_packets(REQ_DEV_NAME, text) {
            assert_eq!(packet[0], REQ_TAG | REQ_DEV_NAME);
            assert!(
                result.is_none(),
                "the transfer ended before its last packet"
            );
            result = reader.push(&packet).unwrap();
        }
        result.expect("the transfer ended without a string")
    }

    #[test]
    fn a_short_string_fits_in_one_packet() {
        assert_eq!(string_packets(REQ_SET_NAME, "printCAD").len(), 1);
        assert_eq!(round_trip("printCAD"), "printCAD");
    }

    #[test]
    fn an_empty_string_still_takes_one_packet() {
        let packets = string_packets(REQ_SET_NAME, "");
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][7], 0);
        assert_eq!(round_trip(""), "");
    }

    #[test]
    fn a_long_string_spans_packets() {
        let text = "3Dconnexion SpaceMouse Wireless (cabled) and then some more";
        let packets = string_packets(REQ_DEV_NAME, text);
        assert_eq!(packets.len(), text.len().div_ceil(CHUNK));
        assert_eq!(packets[0][7], text.len() as i32);
        assert_ne!(packets[1][7] & CONT_BIT, 0);
        assert_eq!(round_trip(text), text);
    }

    #[test]
    fn a_string_exactly_one_chunk_long_takes_one_packet() {
        let text = "a".repeat(CHUNK);
        assert_eq!(string_packets(REQ_DEV_NAME, &text).len(), 1);
        assert_eq!(round_trip(&text), text);
    }

    #[test]
    fn a_continuation_without_a_first_packet_is_rejected() {
        let mut reader = StringReader::default();
        let mut words = [0i32; 8];
        words[7] = 4 | CONT_BIT;
        assert!(reader.push(&words).is_err());
    }

    #[test]
    fn a_chunk_with_the_wrong_remainder_is_rejected() {
        let mut reader = StringReader::default();
        let packets = string_packets(REQ_DEV_NAME, &"b".repeat(40));
        reader.push(&packets[0]).unwrap();
        let mut bad = packets[1];
        bad[7] = 99 | CONT_BIT;
        assert!(reader.push(&bad).is_err());
    }

    #[test]
    fn a_refusal_surfaces_as_such() {
        let mut reader = StringReader::default();
        let mut words = [0i32; 8];
        words[7] = -1;
        assert!(matches!(reader.push(&words), Err(Error::Refused)));

        let mut reply = request_packet(REQ_DEV_NAXES, &[]);
        reply[7] = -1;
        assert!(matches!(
            reply_status(REQ_DEV_NAXES, &reply),
            Err(Error::Refused)
        ));
    }

    #[test]
    fn a_reply_to_another_request_is_rejected() {
        let reply = request_packet(REQ_DEV_NAXES, &[]);
        assert!(reply_status(REQ_DEV_NBUTTONS, &reply).is_err());
        assert!(reply_status(REQ_DEV_NAXES, &reply).is_ok());
    }
}
