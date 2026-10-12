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
pub fn corners(image: &ReferenceImage, aspect: f32) -> [Vec2D; 4] {
    let (w, h) = (image.width / 2.0, image.width * aspect / 2.0);
    let (sin, cos) = image.angle_deg.to_radians().sin_cos();
    let at = |x: f32, y: f32| {
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
    pub length: f32,
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
        let quarter = std::f32::consts::FRAC_PI_4;
        let turn = (along / quarter).round() * quarter - along;
        let rotation = glam::Mat2::from_angle(turn);
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
        ..image.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> ReferenceImage {
        ReferenceImage {
            id: Uuid::nil(),
            asset: Uuid::nil(),
            center: Vec2D::new(10.0, 0.0),
            width: 40.0,
            angle_deg: 0.0,
            opacity: 0.5,
        }
    }

    fn close(a: Vec2D, x: f32, y: f32) -> bool {
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
        let tilt = 5f32.to_radians();
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
            let local = glam::Mat2::from_angle(-old.angle_deg.to_radians())
                * (p.to_glam() - old.center.to_glam());
            Vec2D::from_glam(
                new.center.to_glam() + glam::Mat2::from_angle(new.angle_deg.to_radians()) * local,
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
