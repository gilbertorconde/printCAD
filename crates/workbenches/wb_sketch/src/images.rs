//! Reference images: pictures on the sketch plane to draw over. Their
//! bytes are document assets; decoded once each, they are laid over the
//! viewport while their sketch is edited.

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::sketch::{ReferenceImage, Vec2D};

/// The longest side a picture is shown at, in pixels: plenty to trace, and
/// a texture every GPU takes.
const LONGEST_PX: u32 = 2048;

/// A decoded picture, RGBA rows from the top.
#[derive(Debug)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// Decode a PNG or JPEG file's bytes, no larger than [`LONGEST_PX`].
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let picture = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let picture = if picture.width().max(picture.height()) > LONGEST_PX {
        picture.resize(
            LONGEST_PX,
            LONGEST_PX,
            image::imageops::FilterType::Triangle,
        )
    } else {
        picture
    };
    let rgba = picture.to_rgba8();
    Ok(Decoded {
        width: rgba.width(),
        height: rgba.height(),
        rgba: Arc::new(rgba.into_raw()),
    })
}

/// Pictures decoded so far, by asset: `None` for one that would not decode.
#[derive(Default)]
pub struct Cache(std::cell::RefCell<HashMap<Uuid, Option<Arc<Decoded>>>>);

impl Cache {
    /// The asset's picture, decoded the first time it is asked for.
    pub fn get(
        &self,
        asset: Uuid,
        bytes: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Option<Arc<Decoded>> {
        self.0
            .borrow_mut()
            .entry(asset)
            .or_insert_with(|| bytes().and_then(|b| decode(&b).ok()).map(Arc::new))
            .clone()
    }
}

/// The picture's corners in sketch coordinates: top left, top right,
/// bottom right, bottom left, turned about its middle.
pub fn corners(image: &ReferenceImage, aspect: f64) -> [Vec2D; 4] {
    let (w, h) = (image.width / 2.0, image.width * aspect / 2.0);
    let (sin, cos) = image.angle_deg.to_radians().sin_cos();
    let at = |x: f64, y: f64| {
        Vec2D::new(
            image.center.x + x * cos - y * sin,
            image.center.y + x * sin + y * cos,
        )
    };
    [at(-w, h), at(w, h), at(w, -h), at(-w, -h)]
}

/// How a picture is calibrated: the line between two points picked on it
/// made `length` long, the picture scaled about the first point; with
/// `straighten`, turned about it so the line lies along the nearest 45°
/// direction; with `centre`, moved so the line's middle is the sketch's
/// origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    pub from: Vec2D,
    pub to: Vec2D,
    pub length: f64,
    pub straighten: bool,
    pub centre: bool,
}

/// `image` placed as `calibration` says; `None` when the two points are
/// one or the length is not a positive size.
pub fn calibrate(image: &ReferenceImage, calibration: &Calibration) -> Option<ReferenceImage> {
    let (a, b) = (calibration.from.to_glam(), calibration.to.to_glam());
    let picked = (b - a).length();
    if picked < 1e-6 || !(calibration.length > 0.0 && calibration.length.is_finite()) {
        return None;
    }
    let scale = calibration.length / picked;
    let mut center = a + (image.center.to_glam() - a) * scale;
    let mut angle_deg = image.angle_deg;
    let mut line = (b - a) * scale;
    if calibration.straighten {
        let along = line.y.atan2(line.x);
        let quarter = std::f64::consts::FRAC_PI_4;
        let turn = (along / quarter).round() * quarter - along;
        let rotation = glam::DMat2::from_angle(turn);
        center = a + rotation * (center - a);
        line = rotation * line;
        angle_deg += turn.to_degrees();
    }
    if calibration.centre {
        center -= a + line * 0.5;
    }
    Some(ReferenceImage {
        center: Vec2D::from_glam(center),
        width: image.width * scale,
        angle_deg,
        scale: crate::sketch::ImageScale::Calibrated {
            by: "a distance".into(),
        },
        ..image.clone()
    })
}

/// The picture scaled about `about` (sketch coordinates) so a circle on
/// it measured at `measured` across is `known` across.
pub fn calibrate_by_circle(
    image: &ReferenceImage,
    about: Vec2D,
    measured: f64,
    known: f64,
) -> Option<ReferenceImage> {
    if !(measured > 0.0 && known > 0.0 && known.is_finite()) {
        return None;
    }
    let k = known / measured;
    let c = image.center;
    Some(ReferenceImage {
        center: Vec2D::new(about.x + (c.x - about.x) * k, about.y + (c.y - about.y) * k),
        width: image.width * k,
        scale: crate::sketch::ImageScale::Calibrated {
            by: format!("a circle of {known} mm"),
        },
        ..image.clone()
    })
}

/// The resolution a PNG (its `pHYs` chunk) or JPEG (its JFIF header)
/// states, in dots per inch along its rows.
pub fn dpi(bytes: &[u8]) -> Option<f64> {
    let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut at = 8;
        while at + 8 <= bytes.len() {
            let len = be32(&bytes[at..]) as usize;
            let kind = &bytes[at + 4..at + 8];
            let data = bytes.get(at + 8..at + 8 + len)?;
            if kind == b"pHYs" && len >= 9 {
                // Pixels per metre, when the unit byte says metres.
                return (data[8] == 1 && be32(data) > 0).then(|| f64::from(be32(data)) * 0.0254);
            }
            if kind == b"IDAT" || kind == b"IEND" {
                return None;
            }
            at += 12 + len;
        }
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut at = 2;
        while at + 4 <= bytes.len() && bytes[at] == 0xFF {
            let marker = bytes[at + 1];
            let len = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
            let data = bytes.get(at + 4..at + 2 + len)?;
            if marker == 0xE0 && data.starts_with(b"JFIF\0") && data.len() >= 12 {
                let x = f64::from(u16::from_be_bytes([data[8], data[9]]));
                return match data[7] {
                    1 if x > 0.0 => Some(x),
                    2 if x > 0.0 => Some(x * 2.54),
                    _ => None,
                };
            }
            if marker == 0xDA {
                return None;
            }
            at += 2 + len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG's pHYs chunk and a JPEG's JFIF header give the resolution
    /// they state; a file stating none gives none.
    #[test]
    fn a_file_states_its_resolution() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(9u32.to_be_bytes());
        png.extend(b"pHYs");
        png.extend(11_811u32.to_be_bytes());
        png.extend(11_811u32.to_be_bytes());
        png.extend([1, 0, 0, 0, 0]);
        assert!((dpi(&png).unwrap() - 300.0).abs() < 0.01);
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 2, 0, 100, 0, 100, 0, 0,
        ];
        assert!(
            (dpi(&jpeg).unwrap() - 254.0).abs() < 1e-9,
            "dots per centimetre"
        );
        assert_eq!(dpi(b"\x89PNG\r\n\x1a\n"), None);
    }

    /// A circle measured 9.8 across that is 10 scales the picture by
    /// 10 / 9.8 about the circle's centre, which stays put.
    #[test]
    fn a_known_circle_calibrates_a_picture() {
        let image = picture();
        let about = Vec2D::new(3.0, -2.0);
        let placed = calibrate_by_circle(&image, about, 9.8, 10.0).unwrap();
        let k = 10.0 / 9.8;
        assert!((placed.width - image.width * k).abs() < 1e-12);
        let moved =
            |p: Vec2D| Vec2D::new(about.x + (p.x - about.x) * k, about.y + (p.y - about.y) * k);
        assert!((placed.center.x - moved(image.center).x).abs() < 1e-12);
        assert!(matches!(
            placed.scale,
            crate::sketch::ImageScale::Calibrated { .. }
        ));
        assert!(calibrate_by_circle(&image, about, 0.0, 10.0).is_none());
    }

    fn picture() -> ReferenceImage {
        ReferenceImage {
            id: Uuid::nil(),
            asset: Uuid::nil(),
            center: Vec2D::new(10.0, 0.0),
            width: 40.0,
            angle_deg: 0.0,
            opacity: 0.5,
            scale: Default::default(),
        }
    }

    fn close(a: Vec2D, x: f64, y: f64) -> bool {
        (a.x - x).abs() < 1e-3 && (a.y - y).abs() < 1e-3
    }

    /// Two points 10 apart said to be 25: the picture grows 2.5 times
    /// about the first, which stays where it is.
    #[test]
    fn calibration_scales_about_the_first_point() {
        let done = calibrate(
            &picture(),
            &Calibration {
                from: Vec2D::new(0.0, 0.0),
                to: Vec2D::new(10.0, 0.0),
                length: 25.0,
                straighten: false,
                centre: false,
            },
        )
        .unwrap();
        assert!((done.width - 100.0).abs() < 1e-3);
        assert!(close(done.center, 25.0, 0.0), "{:?}", done.center);
    }

    /// A line picked a few degrees off level is turned level, and centred,
    /// its middle lands on the origin; the picture turns with it.
    #[test]
    fn calibration_straightens_and_centres_the_line() {
        let tilt = 5f64.to_radians();
        let (from, to) = (
            Vec2D::new(2.0, 3.0),
            Vec2D::new(2.0 + 10.0 * tilt.cos(), 3.0 + 10.0 * tilt.sin()),
        );
        let calibration = Calibration {
            from,
            to,
            length: 10.0,
            straighten: true,
            centre: true,
        };
        let done = calibrate(&picture(), &calibration).unwrap();
        assert!((done.angle_deg + 5.0).abs() < 1e-3, "{}", done.angle_deg);
        assert!((done.width - 40.0).abs() < 1e-3);
        // Where the picked points end up: the picture's own frame carried
        // through the same turn and move.
        let carried = |p: Vec2D| {
            let (old, new) = (picture(), &done);
            let local = glam::DMat2::from_angle(-old.angle_deg.to_radians())
                * (p.to_glam() - old.center.to_glam());
            Vec2D::from_glam(
                new.center.to_glam() + glam::DMat2::from_angle(new.angle_deg.to_radians()) * local,
            )
        };
        assert!(close(carried(from), -5.0, 0.0), "{:?}", carried(from));
        assert!(close(carried(to), 5.0, 0.0), "{:?}", carried(to));
    }

    #[test]
    fn calibration_refuses_one_point_or_no_length() {
        let at = Vec2D::new(1.0, 1.0);
        let one = Calibration {
            from: at,
            to: at,
            length: 5.0,
            straighten: false,
            centre: false,
        };
        assert!(calibrate(&picture(), &one).is_none());
        let none = Calibration {
            to: Vec2D::new(4.0, 1.0),
            length: 0.0,
            ..one
        };
        assert!(calibrate(&picture(), &none).is_none());
    }

    /// A picture keeps its proportions and turns about its middle.
    #[test]
    fn a_picture_lies_where_it_is_placed() {
        let image = ReferenceImage {
            id: Uuid::nil(),
            asset: Uuid::nil(),
            center: Vec2D::new(10.0, 5.0),
            width: 40.0,
            angle_deg: 90.0,
            opacity: 0.5,
            scale: Default::default(),
        };
        // Twice as wide as tall: 40 × 20, turned a quarter.
        let [top_left, _, bottom_right, _] = corners(&image, 0.5);
        assert!(
            (top_left.x - 0.0).abs() < 1e-4 && (top_left.y - -15.0).abs() < 1e-4,
            "{top_left:?}"
        );
        assert!((bottom_right.x - 20.0).abs() < 1e-4 && (bottom_right.y - 25.0).abs() < 1e-4);
    }

    /// A PNG decodes to its size and pixels.
    #[test]
    fn a_png_decodes() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(&png).unwrap();
        assert_eq!((decoded.width, decoded.height), (3, 2));
        assert_eq!(&decoded.rgba[..4], &[10, 20, 30, 255]);
        assert!(decode(b"not a picture").is_err());
    }
}
