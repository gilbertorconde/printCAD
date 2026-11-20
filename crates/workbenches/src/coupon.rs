//! The calibration coupon: a part the host offers ready-made (a start
//! page card and the `doc.coupon` script call), built from the sketcher's
//! and the Design bench's features so the host never names them.

use core_document::{BodyId, Document, FeatureId};
use sketch_model::SketchFeature;
use sketch_model::sketch::{Circle, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};
use wb_design::{DesignFeature, ExtrudeMode};

/// The calibration coupon's sizes, mm.
#[derive(Debug, Clone, PartialEq)]
pub struct CouponSizes {
    /// The holes' and pins' diameters, one of each per size.
    pub diameters: Vec<f64>,
    /// The ladder's slot widths.
    pub gaps: Vec<f64>,
    /// The fan's overhangs, degrees past vertical.
    pub overhangs: Vec<f64>,
}

impl Default for CouponSizes {
    fn default() -> Self {
        Self {
            diameters: vec![3.0, 4.0, 5.0, 6.0, 8.0],
            gaps: vec![0.3, 0.4, 0.5, 0.6, 0.8],
            overhangs: vec![30.0, 45.0, 60.0, 70.0],
        }
    }
}

/// The coupon's plate thickness and pin height, mm.
const COUPON_PLATE: f64 = 3.0;
const COUPON_PIN: f64 = 6.0;

/// What a coupon was built as, for checking a print of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Coupon {
    pub body: BodyId,
    /// The plate's size, mm.
    pub width: f64,
    pub depth: f64,
    /// Each hole's centre and diameter (the plate's mid-height), and each
    /// pin's (its mid-height).
    pub holes: Vec<([f64; 3], f64)>,
    pub pins: Vec<([f64; 3], f64)>,
    /// Each slot's middle and width.
    pub gaps: Vec<([f64; 3], f64)>,
    /// The face each overhang leans out on: a point in its middle and
    /// its angle past vertical.
    pub overhangs: Vec<([f64; 3], f64)>,
    /// The features made, in order.
    pub features: Vec<FeatureId>,
}

/// A sketch's closed polygon through `corners`.
fn polygon(sketch: &mut Sketch, corners: &[(f64, f64)]) {
    let ids: Vec<_> = corners
        .iter()
        .map(|(x, y)| sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(*x, *y)))))
        .collect();
    for k in 0..ids.len() {
        sketch.add_geometry(GeometryElement::Line(Line::new(
            ids[k],
            ids[(k + 1) % ids.len()],
        )));
    }
}

fn circle(sketch: &mut Sketch, x: f64, y: f64, radius: f64) {
    let centre = sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))));
    sketch.add_geometry(GeometryElement::Circle(Circle::new(centre, radius)));
}

fn pad_of(sketch: FeatureId, length: f64) -> DesignFeature {
    DesignFeature::Pad {
        regions: Vec::new(),
        profile_borrowed: None,
        extras: Default::default(),
        refine: true,
        boolean_tolerance: None,
        sketch: Some(sketch),
        length: length as f32,
        reversed: false,
        symmetric: false,
        mode: ExtrudeMode::Dimension,
        length2: 0.0,
        taper_deg: 0.0,
        up_to_face: None,
        up_to_offset: 0.0,
        profile_face: None,
        direction: Default::default(),
        up_to_shape: Vec::new(),
        mode2: None,
        up_to_face2: None,
        up_to_offset2: 0.0,
        up_to_shape2: Vec::new(),
    }
}

/// A calibration coupon in a body of its own: a plate with a row of
/// holes through it and a ladder of slots, a row of pins standing on it,
/// and a fan of overhangs along its back edge, each leaning out further.
pub fn coupon(document: &mut Document, sizes: &CouponSizes) -> Result<Coupon, String> {
    let (t, tall) = (COUPON_PLATE, COUPON_PIN);
    if sizes.diameters.is_empty() || sizes.diameters.iter().any(|d| !(1.0..=20.0).contains(d)) {
        return Err("each diameter is from 1 to 20 mm".into());
    }
    if sizes.gaps.iter().any(|g| !(0.05..=3.0).contains(g)) {
        return Err("each gap is from 0.05 to 3 mm".into());
    }
    if sizes.overhangs.iter().any(|a| !(0.0..=80.0).contains(a)) {
        return Err("each overhang is from 0 to 80 degrees".into());
    }
    let widest = sizes.diameters.iter().copied().fold(0.0, f64::max);
    let pitch = widest + 5.0;
    let fan_height = 4.0;
    let tooth = |a: f64| 3.0 + fan_height * a.to_radians().tan();
    let rows = [
        sizes.diameters.len() as f64 * pitch + 4.0,
        sizes.gaps.iter().map(|g| g + 4.0).sum::<f64>() + 8.0,
        sizes.overhangs.iter().map(|a| tooth(*a) + 3.0).sum::<f64>() + 6.0,
    ];
    let width = rows.into_iter().fold(0.0, f64::max).ceil();
    let (hole_y, pin_y) = (2.0 + widest / 2.0, 6.0 + widest * 1.5);
    let gap_y = pin_y + widest / 2.0 + 4.0;
    let fan_y = gap_y + 10.0;
    let depth = fan_y + 6.0;
    let x = |k: usize| 2.0 + pitch * (k as f64 + 0.5);

    let body = document.create_body(None);
    document.rename_body(body, "Coupon");
    let mut features = Vec::new();
    let add_sketch = |document: &mut Document, sketch: Sketch, name: &str| {
        let plane = sketch.plane;
        document
            .add_feature_in_body(SketchFeature::new(sketch, plane), name.into(), Some(body))
            .map_err(|e| format!("{name}: {e}"))
    };

    // The plate, its holes and slots in one profile.
    let mut plate = Sketch::new("Plate");
    polygon(
        &mut plate,
        &[(0.0, 0.0), (width, 0.0), (width, depth), (0.0, depth)],
    );
    let mut holes = Vec::new();
    for (k, d) in sizes.diameters.iter().enumerate() {
        circle(&mut plate, x(k), hole_y, d / 2.0);
        holes.push(([x(k), hole_y, t / 2.0], *d));
    }
    let mut gaps = Vec::new();
    let mut at = 6.0;
    for g in &sizes.gaps {
        polygon(
            &mut plate,
            &[
                (at, gap_y),
                (at + g, gap_y),
                (at + g, gap_y + 6.0),
                (at, gap_y + 6.0),
            ],
        );
        gaps.push(([at + g / 2.0, gap_y + 3.0, t / 2.0], *g));
        at += g + 4.0;
    }
    let sketch = add_sketch(document, plate, "Plate")?;
    document.set_feature_visible(sketch, false);
    let pad = document
        .add_feature_in_body(pad_of(sketch, t), "Plate".into(), Some(body))
        .map_err(|e| format!("plate: {e}"))?;
    features.extend([sketch, pad]);

    // The pins, standing on the plate.
    let mut pin_sketch = Sketch::new("Pins");
    pin_sketch.plane.origin = [0.0, 0.0, t];
    let mut pins = Vec::new();
    for (k, d) in sizes.diameters.iter().enumerate() {
        circle(&mut pin_sketch, x(k), pin_y, d / 2.0);
        pins.push(([x(k), pin_y, t + tall / 2.0], *d));
    }
    let sketch = add_sketch(document, pin_sketch, "Pins")?;
    document.set_feature_visible(sketch, false);
    let pad = document
        .add_feature_in_body(pad_of(sketch, tall), "Pins".into(), Some(body))
        .map_err(|e| format!("pins: {e}"))?;
    features.extend([sketch, pad]);

    // The fan: teeth along the back edge, each one's right side leaning
    // out past vertical by its angle.
    let mut overhangs = Vec::new();
    if !sizes.overhangs.is_empty() {
        let mut fan = Sketch::new("Overhangs");
        fan.plane = SketchPlane {
            origin: [0.0, depth, 0.0],
            ..SketchPlane::xz()
        };
        let mut at = 4.0;
        for a in &sizes.overhangs {
            let lean = fan_height * a.to_radians().tan();
            polygon(
                &mut fan,
                &[
                    (at, t),
                    (at + 3.0, t),
                    (at + 3.0 + lean, t + fan_height),
                    (at + lean, t + fan_height),
                ],
            );
            overhangs.push((
                [at + 3.0 + lean / 2.0, depth - 3.0, t + fan_height / 2.0],
                *a,
            ));
            at += tooth(*a) + 3.0;
        }
        let sketch = add_sketch(document, fan, "Overhangs")?;
        document.set_feature_visible(sketch, false);
        let pad = document
            .add_feature_in_body(pad_of(sketch, 6.0), "Overhangs".into(), Some(body))
            .map_err(|e| format!("overhangs: {e}"))?;
        features.extend([sketch, pad]);
    }
    for f in &features {
        document.mark_feature_dirty(*f);
    }
    Ok(Coupon {
        body,
        width,
        depth,
        holes,
        pins,
        gaps,
        overhangs,
        features,
    })
}
