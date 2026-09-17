//! Packet framing and event decoding.
//!
//! Every packet on the socket is exactly 32 bytes: eight native-endian 32-bit
//! words. Word 0 tells the two kinds apart. An event names its type there, in
//! `0..MAX_EVENT_TYPE`; a request and its echoed response carry
//! [`REQ_TAG`](crate::request::REQ_TAG) in the high half, so the two can never
//! be confused for one another.

use crate::{Error, Event, Motion};

pub(crate) const PACKET_WORDS: usize = 8;
pub(crate) const PACKET_BYTES: usize = PACKET_WORDS * 4;

const UEV_MOTION: i32 = 0;
const UEV_PRESS: i32 = 1;
const UEV_RELEASE: i32 = 2;
const UEV_DEV: i32 = 3;
const UEV_CFG: i32 = 4;
const UEV_RAWAXIS: i32 = 5;
const UEV_RAWBUTTON: i32 = 6;

/// One past the last event type the daemon can send.
pub(crate) const MAX_EVENT_TYPE: i32 = 7;

/// The daemon's device-change opcodes.
const DEV_ADD: i32 = 0;

pub(crate) type Words = [i32; PACKET_WORDS];

pub(crate) fn words_from_bytes(bytes: &[u8; PACKET_BYTES]) -> Words {
    let mut words = [0i32; PACKET_WORDS];
    let (chunks, _) = bytes.as_chunks::<4>();
    for (word, chunk) in words.iter_mut().zip(chunks) {
        *word = i32::from_ne_bytes(*chunk);
    }
    words
}

pub(crate) fn bytes_from_words(words: &Words) -> [u8; PACKET_BYTES] {
    let mut bytes = [0u8; PACKET_BYTES];
    let (chunks, _) = bytes.as_chunks_mut::<4>();
    for (word, chunk) in words.iter().zip(chunks) {
        *chunk = word.to_ne_bytes();
    }
    bytes
}

/// Whether a packet is an event rather than a response to a request.
pub(crate) fn is_event(words: &Words) -> bool {
    (0..MAX_EVENT_TYPE).contains(&words[0])
}

fn index(raw: i32) -> Result<u32, Error> {
    u32::try_from(raw).map_err(|_| Error::Protocol("negative index in an event packet"))
}

fn usb_id(vendor: i32, product: i32) -> Option<(u16, u16)> {
    let vendor = u16::try_from(vendor).ok()?;
    let product = u16::try_from(product).ok()?;
    (vendor != 0 || product != 0).then_some((vendor, product))
}

pub(crate) fn decode_event(words: &Words) -> Result<Event, Error> {
    match words[0] {
        UEV_MOTION => Ok(Event::Motion(Motion {
            translate: [words[1], words[2], words[3]],
            rotate: [words[4], words[5], words[6]],
            period_ms: words[7].max(0) as u32,
        })),
        UEV_PRESS | UEV_RELEASE => Ok(Event::Button {
            index: index(words[1])?,
            pressed: words[0] == UEV_PRESS,
        }),
        UEV_DEV => Ok(Event::Device {
            added: words[1] == DEV_ADD,
            id: words[2],
            device_type: words[3],
            usb_id: usb_id(words[4], words[5]),
        }),
        UEV_CFG => Ok(Event::Config { item: words[1] }),
        UEV_RAWAXIS => Ok(Event::RawAxis {
            index: index(words[1])?,
            value: words[2],
        }),
        UEV_RAWBUTTON => Ok(Event::RawButton {
            index: index(words[1])?,
            pressed: words[2] != 0,
        }),
        _ => Err(Error::Protocol("unknown event type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(words: Words) -> Words {
        words_from_bytes(&bytes_from_words(&words))
    }

    #[test]
    fn words_survive_a_round_trip_through_bytes() {
        let words = [0, 1, -2, 3, -4, 5, -6, 7];
        assert_eq!(packet(words), words);
    }

    #[test]
    fn motion_carries_six_axes_and_a_period() {
        let event = decode_event(&packet([UEV_MOTION, 1, -2, 3, -4, 5, -6, 16])).unwrap();
        assert_eq!(
            event,
            Event::Motion(Motion {
                translate: [1, -2, 3],
                rotate: [-4, 5, -6],
                period_ms: 16,
            })
        );
    }

    #[test]
    fn press_and_release_decode_to_one_variant() {
        let press = decode_event(&packet([UEV_PRESS, 2, 1, 0, 0, 0, 0, 0])).unwrap();
        let release = decode_event(&packet([UEV_RELEASE, 2, 0, 0, 0, 0, 0, 0])).unwrap();
        assert_eq!(
            press,
            Event::Button {
                index: 2,
                pressed: true
            }
        );
        assert_eq!(
            release,
            Event::Button {
                index: 2,
                pressed: false
            }
        );
    }

    #[test]
    fn a_serial_device_reports_no_usb_id() {
        let event = decode_event(&packet([UEV_DEV, DEV_ADD, 7, 0x100, 0, 0, 0, 0])).unwrap();
        assert_eq!(
            event,
            Event::Device {
                added: true,
                id: 7,
                device_type: 0x100,
                usb_id: None,
            }
        );
    }

    #[test]
    fn a_usb_device_reports_its_vendor_and_product() {
        let event = decode_event(&packet([UEV_DEV, 1, 3, 0x206, 0x256f, 0xc62e, 0, 0])).unwrap();
        assert_eq!(
            event,
            Event::Device {
                added: false,
                id: 3,
                device_type: 0x206,
                usb_id: Some((0x256f, 0xc62e)),
            }
        );
    }

    #[test]
    fn an_out_of_range_type_is_rejected() {
        assert!(decode_event(&packet([MAX_EVENT_TYPE, 0, 0, 0, 0, 0, 0, 0])).is_err());
        assert!(decode_event(&packet([-1, 0, 0, 0, 0, 0, 0, 0])).is_err());
    }

    #[test]
    fn requests_are_never_mistaken_for_events() {
        assert!(is_event(&[UEV_RAWBUTTON, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!is_event(&[
            crate::request::REQ_TAG | crate::request::REQ_DEV_NAME,
            0,
            0,
            0,
            0,
            0,
            0,
            0
        ]));
    }
}
