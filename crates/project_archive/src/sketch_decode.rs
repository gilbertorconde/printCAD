// Copyright (c) 2026 Stan Yolo
// SPDX-License-Identifier: MIT OR Apache-2.0
// See ../NOTICE for the retained permission notice.
//! A `Sketcher::SketchObject`: geometry and constraints in the archive's vocabulary,
//! before translation into the application model.

use std::f64::consts::PI;

use crate::coordinates::Point2;

use crate::archive::FcstdError;
use crate::document_xml::{FcObject, XmlElement, check_finite};

/// A constraint's geometry index: nonnegative means list order, `-1` is the H axis,
/// `-2` the V axis, values at most `-3` external geometry, and `-2000` absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FcGeoIndex(pub i64);

impl FcGeoIndex {
    pub const H_AXIS: Self = Self(-1);
    pub const V_AXIS: Self = Self(-2);
    pub const NONE: Self = Self(-2000);

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    pub fn is_external(self) -> bool {
        self.0 <= -3 && !self.is_none()
    }
}

/// `Sketcher::PointPos`: the part of an element referenced by a constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FcPointPos {
    /// The whole element (edge).
    Edge,
    Start,
    End,
    /// A circle's or arc's center.
    Mid,
}

impl FcPointPos {
    fn parse(object: &FcObject, value: i64) -> Result<Self, FcstdError> {
        match value {
            0 => Ok(Self::Edge),
            1 => Ok(Self::Start),
            2 => Ok(Self::End),
            3 => Ok(Self::Mid),
            other => Err(object.format_error(format!("constraint point position {other}"))),
        }
    }
}

/// `Sketcher::ConstraintType` (`src/Mod/Sketcher/App/Constraint.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FcConstraintType {
    Coincident,
    Horizontal,
    Vertical,
    Parallel,
    Tangent,
    Distance,
    DistanceX,
    DistanceY,
    Angle,
    Perpendicular,
    Radius,
    Equal,
    PointOnObject,
    Symmetric,
    InternalAlignment,
    SnellsLaw,
    Block,
    Diameter,
    Weight,
    /// A code absent from this version's enumeration.
    Unknown(i64),
}

impl FcConstraintType {
    fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Coincident,
            2 => Self::Horizontal,
            3 => Self::Vertical,
            4 => Self::Parallel,
            5 => Self::Tangent,
            6 => Self::Distance,
            7 => Self::DistanceX,
            8 => Self::DistanceY,
            9 => Self::Angle,
            10 => Self::Perpendicular,
            11 => Self::Radius,
            12 => Self::Equal,
            13 => Self::PointOnObject,
            14 => Self::Symmetric,
            15 => Self::InternalAlignment,
            16 => Self::SnellsLaw,
            17 => Self::Block,
            18 => Self::Diameter,
            19 => Self::Weight,
            other => Self::Unknown(other),
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::Unknown(code) => format!("type {code}"),
            other => format!("{other:?}"),
        }
    }
}

/// An element's shape in local sketch coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum FcShape {
    Line {
        start: Point2,
        end: Point2,
    },
    Circle {
        center: Point2,
        radius: f64,
    },
    /// A canonical sketch arc runs counterclockwise from `start` to `end`.
    /// For a `-Z` normal, `emulateCCW` swaps endpoints; constraints reference this
    /// canonical order, so decoding must perform the same swap.
    Arc {
        center: Point2,
        radius: f64,
        start: Point2,
        end: Point2,
    },
    Point(Point2),
    /// An unsupported archive type, such as `Part::GeomBSplineCurve`.
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FcGeometry {
    pub shape: FcShape,
    pub construction: bool,
    /// `Sketcher::InternalType`: internal role, such as a spline pole or ellipse axis.
    /// Zero means ordinary geometry.
    pub internal: i64,
}

/// Default `Sketcher::Constraint::LabelDistance` when the file omits it (`Constraint.h`).
const FC_DEFAULT_LABEL_DISTANCE: f64 = 10.0;

/// Bit index of `Sketcher::GeometryMode::Construction` in `geometryModeFlags`
/// (`SketchGeometryExtension.h`).
const FC_GEOMETRY_MODE_CONSTRUCTION: usize = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct FcConstraint {
    pub kind: FcConstraintType,
    /// Constraint value: lengths in millimetres, angles in radians.
    pub value: f64,
    pub first: FcGeoIndex,
    pub first_pos: FcPointPos,
    pub second: FcGeoIndex,
    pub second_pos: FcPointPos,
    pub third: FcGeoIndex,
    pub third_pos: FcPointPos,
    pub driving: bool,
    pub active: bool,
    /// The dimension label's offset from geometry.
    pub label_distance: f64,
    /// Constraint name used by expressions (`Sketch.Constraints.width`);
    /// an empty string means unnamed.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FcSketch {
    pub geometry: Vec<FcGeometry>,
    pub constraints: Vec<FcConstraint>,
    /// External geometry beyond the H and V axes at the start of `ExternalGeo`.
    pub external_count: usize,
}

impl FcSketch {
    pub fn decode(object: &FcObject) -> Result<Self, FcstdError> {
        let geometry = geometry_list(object, "Geometry")?
            .iter()
            .map(|element| geometry(object, element))
            .collect::<Result<Vec<_>, _>>()?;
        let external_count = if object.has("ExternalGeo") {
            geometry_list(object, "ExternalGeo")?
                .len()
                .saturating_sub(2)
        } else {
            0
        };
        let constraints = constraint_list(object)?
            .iter()
            .map(|element| constraint(object, element))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            geometry,
            constraints,
            external_count,
        })
    }
}

fn geometry_list<'a>(object: &'a FcObject, name: &str) -> Result<Vec<&'a XmlElement>, FcstdError> {
    let list = object
        .property(name)?
        .value
        .iter()
        .find(|element| element.tag == "GeometryList")
        .ok_or_else(|| object.format_error(format!("{name} has no <GeometryList>")))?;
    Ok(list.children_named("Geometry").collect())
}

fn constraint_list(object: &FcObject) -> Result<Vec<&XmlElement>, FcstdError> {
    let list = object
        .property("Constraints")?
        .value
        .iter()
        .find(|element| element.tag == "ConstraintList")
        .ok_or_else(|| object.format_error("Constraints has no <ConstraintList>"))?;
    Ok(list.children_named("Constrain").collect())
}

fn geometry(object: &FcObject, element: &XmlElement) -> Result<FcGeometry, FcstdError> {
    let type_name = element
        .attr("type")
        .ok_or_else(|| object.format_error("<Geometry> without type"))?;
    let sketch_extension = element
        .child("GeoExtensions")
        .into_iter()
        .flat_map(|ext| ext.children_named("GeoExtension"))
        .find(|ext| ext.attr("type") == Some("Sketcher::SketchGeometryExtension"));
    // Construction has version-dependent encodings:
    // * before 0.20: only `<Construction value>`;
    // * 0.20-0.21: only bit 1 (`GeometryMode::Construction`) of the sketch extension's
    //   `geometryModeFlags`, serialized as a 32-bit string with the high bit first;
    // * 1.x: both the bit and `<Construction>`, written from it by `postSave`.
    // When both occur they must agree, otherwise the archive is inconsistent.
    let legacy = match element.child("Construction").map(|c| c.attr("value")) {
        None => None,
        Some(Some("1")) => Some(true),
        Some(Some("0")) => Some(false),
        Some(other) => {
            return Err(
                object.format_error(format!("geometry {type_name}: construction flag {other:?}"))
            );
        }
    };
    let mode_bit = match sketch_extension.and_then(|ext| ext.attr("geometryModeFlags")) {
        None => None,
        Some(flags) => {
            let bits = flags.as_bytes();
            if bits.len() != 32 || !bits.iter().all(|b| matches!(b, b'0' | b'1')) {
                return Err(object
                    .format_error(format!("geometry {type_name}: geometryModeFlags {flags:?}")));
            }
            Some(bits[31 - FC_GEOMETRY_MODE_CONSTRUCTION] == b'1')
        }
    };
    let construction = match (legacy, mode_bit) {
        (Some(legacy), Some(bit)) if legacy != bit => {
            return Err(object.format_error(format!(
                "geometry {type_name}: <Construction> {legacy} disagrees with geometryModeFlags {bit}"
            )));
        }
        (Some(flag), _) | (None, Some(flag)) => flag,
        (None, None) => {
            return Err(object.format_error(format!("geometry {type_name}: no construction flag")));
        }
    };
    let internal = match sketch_extension.and_then(|ext| ext.attr("internalGeometryType")) {
        Some(text) => text
            .parse()
            .map_err(|_| object.format_error(format!("internalGeometryType {text:?}")))?,
        // Older files without extensions contain no internal geometry roles.
        None => 0,
    };
    let f = |tag: &str, attr: &str| -> Result<f64, FcstdError> {
        let data = element
            .child(tag)
            .ok_or_else(|| object.format_error(format!("geometry {type_name} has no <{tag}>")))?;
        object.attr_f64(type_name, data, attr)
    };
    // Older encodings may omit the unused local Z / normal X,Y components.
    // When present they must describe the same local plane as the stored 2D data.
    let planar = |tag: &str, attrs: &[&str]| -> Result<(), FcstdError> {
        if let Some(data) = element.child(tag) {
            for attr in attrs {
                if data.attr(attr).is_some() {
                    let value = f(tag, attr)?;
                    if value.abs() > 1e-9 {
                        return Err(object.format_error(format!(
                            "{type_name}.{attr}: {value} is outside the local XY plane"
                        )));
                    }
                }
            }
        }
        Ok(())
    };
    match type_name {
        "Part::GeomLineSegment" => planar("LineSegment", &["StartZ", "EndZ"]),
        "Part::GeomPoint" => planar("GeomPoint", &["Z"]),
        "Part::GeomCircle" => planar("Circle", &["CenterZ", "NormalX", "NormalY"]),
        "Part::GeomArcOfCircle" => planar("ArcOfCircle", &["CenterZ", "NormalX", "NormalY"]),
        _ => Ok(()),
    }?;
    let shape = match type_name {
        "Part::GeomLineSegment" => FcShape::Line {
            start: Point2::new(f("LineSegment", "StartX")?, f("LineSegment", "StartY")?),
            end: Point2::new(f("LineSegment", "EndX")?, f("LineSegment", "EndY")?),
        },
        "Part::GeomPoint" => {
            FcShape::Point(Point2::new(f("GeomPoint", "X")?, f("GeomPoint", "Y")?))
        }
        "Part::GeomCircle" => {
            planar_normal(object, type_name, f("Circle", "NormalZ")?)?;
            FcShape::Circle {
                center: Point2::new(f("Circle", "CenterX")?, f("Circle", "CenterY")?),
                radius: f("Circle", "Radius")?,
            }
        }
        "Part::GeomArcOfCircle" => {
            let tag = "ArcOfCircle";
            let center = Point2::new(f(tag, "CenterX")?, f(tag, "CenterY")?);
            let radius = f(tag, "Radius")?;
            let normal_up = planar_normal(object, type_name, f(tag, "NormalZ")?)?;
            // Absence selects the circle frame's reference X direction. An
            // explicit rotation still goes through strict numeric validation.
            let data = element.child(tag).expect("arc fields require ArcOfCircle");
            let angle_xu = match data.attr("AngleXU") {
                None => 0.0,
                Some(_) => f(tag, "AngleXU")?,
            };
            let (start, end) = arc_ends(
                object,
                center,
                radius,
                normal_up,
                angle_xu,
                f(tag, "StartAngle")?,
                f(tag, "EndAngle")?,
            )?;
            FcShape::Arc {
                center,
                radius,
                start,
                end,
            }
        }
        other => FcShape::Other(other.to_owned()),
    };
    Ok(FcGeometry {
        shape,
        construction,
        internal,
    })
}

/// Sketch curves must have a `±Z` normal in their local plane. Returns true for `+Z`.
fn planar_normal(object: &FcObject, type_name: &str, normal_z: f64) -> Result<bool, FcstdError> {
    if (normal_z - 1.0).abs() < 1e-9 {
        Ok(true)
    } else if (normal_z + 1.0).abs() < 1e-9 {
        Ok(false)
    } else {
        Err(object.format_error(format!("{type_name}: normal z {normal_z} is not ±1")))
    }
}

/// Canonical sketch arc endpoints, counterclockwise from start to end.
///
/// The circle frame is `(N, Xd, Yd = N × Xd)` and its parameter point is
/// `C + R(cos t·Xd + sin t·Yd)`. `AngleXU` measures `Xd` about `N` from the frame's
/// reference X direction (`GeomConic::getAngleXU`). That reference is `+X` for
/// `N = +Z` and `-X` for `N = -Z`. In sketch angles about `+Z`:
/// * `N = +Z`: `Xd` has angle `a`, so parameter `t` has angle `a + t`;
/// * `N = -Z`: `Xd` has angle `π - a`, so parameter `t` has angle `π - a - t`.
///
/// For `N = -Z`, the parameter runs clockwise. `getStartPoint(emulateCCW = true)`
/// names its parameter end as the sketch start, preserving the same arc counterclockwise.
fn arc_ends(
    object: &FcObject,
    center: Point2,
    radius: f64,
    normal_up: bool,
    angle_xu: f64,
    start_param: f64,
    end_param: f64,
) -> Result<(Point2, Point2), FcstdError> {
    let at = |angle: f64| -> Result<Point2, FcstdError> {
        check_finite(&object.name, "ArcOfCircle computed angle", angle)?;
        let point = Point2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        check_finite(&object.name, "ArcOfCircle computed endpoint.x", point.x)?;
        check_finite(&object.name, "ArcOfCircle computed endpoint.y", point.y)?;
        Ok(point)
    };
    if normal_up {
        Ok((at(angle_xu + start_param)?, at(angle_xu + end_param)?))
    } else {
        let x_dir = PI - angle_xu;
        Ok((at(x_dir - end_param)?, at(x_dir - start_param)?))
    }
}

fn constraint(object: &FcObject, element: &XmlElement) -> Result<FcConstraint, FcstdError> {
    let what = "Constrain";
    let f = |attr| object.attr_f64(what, element, attr);
    let i = |attr| object.attr_i64(what, element, attr);
    // Older files omit attributes with `Sketcher::Constraint` member defaults
    // (`Constraint::Restore` checks `hasAttribute`). Files before 0.19 omit `IsActive`
    // and all their constraints are active. Absence is valid; present values are strict.
    let flag = |attr: &str, absent: bool| -> Result<bool, FcstdError> {
        match element.attr(attr) {
            None => Ok(absent),
            Some("1") => Ok(true),
            Some("0") => Ok(false),
            other => Err(object.format_error(format!("constraint {attr} {other:?}"))),
        }
    };
    let label_distance = match element.attr("LabelDistance") {
        None => FC_DEFAULT_LABEL_DISTANCE,
        Some(_) => f("LabelDistance")?,
    };
    Ok(FcConstraint {
        kind: FcConstraintType::from_code(i("Type")?),
        value: f("Value")?,
        first: FcGeoIndex(i("First")?),
        first_pos: FcPointPos::parse(object, i("FirstPos")?)?,
        second: FcGeoIndex(i("Second")?),
        second_pos: FcPointPos::parse(object, i("SecondPos")?)?,
        third: FcGeoIndex(i("Third")?),
        third_pos: FcPointPos::parse(object, i("ThirdPos")?)?,
        driving: flag("IsDriving", true)?,
        active: flag("IsActive", true)?,
        label_distance,
        name: element
            .attr("Name")
            .ok_or_else(|| object.format_error("constraint without Name"))?
            .to_owned(),
    })
}
