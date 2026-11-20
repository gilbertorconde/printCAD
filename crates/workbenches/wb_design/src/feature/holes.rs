//! The settings of holes: where one starts, its cut, thread, fit, drill
//! point and nut trap, and the defaults older files lack.

use super::*;

/// Where a hole starts drilling: its sketch's plane carried along the
/// drill onto a datum plane, another sketch's plane or a flat face of the
/// solid, then `offset` further in.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
#[serde(default)]
pub struct HoleStart {
    /// Millimetres further into the material; negative starts above.
    #[walk(length("start_offset", "Start offset"))]
    pub offset: f32,
    /// A datum plane or a sketch whose plane the hole starts on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<FeatureId>,
    /// A flat face of the solid the hole starts on, in place of `at`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face: Option<FacePick>,
}

impl HoleStart {
    /// The hole does not start on its sketch's plane.
    pub fn moves(&self) -> bool {
        self.offset != 0.0 || self.at.is_some() || self.face.is_some()
    }
}

/// A hexagonal pocket for a captive nut, at one end of a hole: sized
/// from the nut of the hole's metric thread, or from an across-flats of
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
#[serde(default)]
pub struct NutTrap {
    /// The nut standard the size comes from.
    #[walk(setting)]
    pub standard: crate::hole_tables::NutStandard,
    /// Across the pocket's flats, mm, in place of the nut's.
    #[walk(length("nut_across_flats", "Nut trap across flats"))]
    pub across_flats: Option<f32>,
    /// Added to the nut's across-flats and to its thickness, mm, so the
    /// nut drops in.
    #[walk(length("nut_clearance", "Nut trap clearance"))]
    pub clearance: f32,
    /// How deep the pocket is, mm; the nut's thickness and the clearance
    /// when not given.
    #[walk(length("nut_depth", "Nut trap depth"))]
    pub depth: Option<f32>,
    /// Which end of the hole it sits at.
    #[walk(setting)]
    pub side: NutSide,
    /// The hexagon turned about the hole's axis, degrees; at 0 a corner
    /// points along the sketch's X.
    #[walk(angle("nut_turn", "Nut trap turn"))]
    pub turn_deg: f32,
}

impl Default for NutTrap {
    fn default() -> Self {
        Self {
            standard: Default::default(),
            across_flats: None,
            clearance: 0.3,
            depth: None,
            side: NutSide::Top,
            turn_deg: 0.0,
        }
    }
}

/// Which end of a hole a nut trap sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Describe)]
pub enum NutSide {
    /// At its mouth, on the sketch's plane.
    #[default]
    Top,
    /// At its bottom, the pocket's floor where the hole's depth ends: a
    /// nut set in while the part prints.
    Bottom,
}

/// A hole's nut trap as a script may give it: whole, `true` for the
/// usual one, or `false` for none.
/// A thread's size as a document or a script gives it, or "Custom" (or
/// an empty name) for none: the thread's own pitch and depth.
pub(super) fn thread_or_custom<'de, D>(deserializer: D) -> Result<Option<ThreadSpec>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(name))
            if name.is_empty() || name.eq_ignore_ascii_case("custom") =>
        {
            Ok(None)
        }
        Some(v) => ThreadSpec::deserialize(v)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

pub(crate) fn nut_trap_or_flag<'de, D>(deserializer: D) -> Result<Option<NutTrap>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Given {
        Flag(bool),
        Trap(NutTrap),
    }
    Ok(match Option::<Given>::deserialize(deserializer)? {
        None | Some(Given::Flag(false)) => None,
        Some(Given::Flag(true)) => Some(NutTrap::default()),
        Some(Given::Trap(trap)) => Some(trap),
    })
}

/// What a hole cuts around its mouth, beyond the drill.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum HoleCut {
    #[default]
    None,
    Counterbore {
        #[walk(length("counterbore_diameter", "Counterbore diameter"))]
        diameter: f32,
        #[walk(length("counterbore_depth", "Counterbore depth"))]
        depth: f32,
    },
    /// A cone at the mouth: `angle_deg` is its included angle, 90° when
    /// none is given.
    Countersink {
        #[walk(length("countersink_diameter", "Countersink diameter"))]
        diameter: f32,
        #[serde(default = "countersink_angle")]
        #[walk(angle("countersink_angle", "Countersink angle"))]
        angle_deg: f32,
    },
    /// A shallow counterbore: a flat seat faced into a rough or curved
    /// surface.
    Spotface {
        #[walk(length("spotface_diameter", "Spotface diameter"))]
        diameter: f32,
        #[walk(length("spotface_depth", "Spotface depth"))]
        depth: f32,
    },
    /// A wider bore ending in a cone that narrows to the hole, as a
    /// stepped drill leaves it: `angle_deg` is the cone's included angle,
    /// the drill's 118° when none is given.
    Counterdrill {
        #[walk(length("counterdrill_diameter", "Counterdrill diameter"))]
        diameter: f32,
        #[walk(length("counterdrill_depth", "Counterdrill depth"))]
        depth: f32,
        #[serde(default = "drill_point_angle")]
        #[walk(angle("counterdrill_angle", "Counterdrill angle"))]
        angle_deg: f32,
    },
    /// The seat of a standard screw's head, sized from the hole's metric
    /// thread size.
    Seat {
        #[walk(setting)]
        seat: crate::hole_tables::ScrewSeat,
    },
}

impl HoleCut {
    pub fn label(&self) -> &'static str {
        match self {
            HoleCut::None => "None",
            HoleCut::Counterbore { .. } => "Counterbore",
            HoleCut::Countersink { .. } => "Countersink",
            HoleCut::Spotface { .. } => "Spotface",
            HoleCut::Counterdrill { .. } => "Counterdrill",
            HoleCut::Seat { seat } => seat.label(),
        }
    }
}

/// How long a hole's modeled thread runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Describe)]
pub enum ThreadLength {
    /// As deep as the thread depth given.
    #[default]
    Given,
    /// The whole depth of the hole.
    HoleDepth,
    /// The hole's depth less the tap's run-out, three pitches.
    RunOut,
}

impl ThreadLength {
    pub const ALL: [ThreadLength; 3] = [
        ThreadLength::Given,
        ThreadLength::HoleDepth,
        ThreadLength::RunOut,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ThreadLength::Given => "Depth given",
            ThreadLength::HoleDepth => "Whole hole",
            ThreadLength::RunOut => "Less the run-out",
        }
    }
}

/// ISO 273 metric clearance style for a threaded hole size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Describe)]
pub enum HoleFit {
    Close,
    #[default]
    Normal,
    Loose,
}

impl HoleFit {
    pub const ALL: [HoleFit; 3] = [HoleFit::Close, HoleFit::Normal, HoleFit::Loose];

    pub fn label(&self) -> &'static str {
        match self {
            HoleFit::Close => "Close",
            HoleFit::Normal => "Normal",
            HoleFit::Loose => "Loose",
        }
    }
}

/// The thread a standards-driven hole is sized from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadSpec {
    pub standard: ThreadStandard,
    /// The size's designation in its standard: "M6", "1/4-20", "1/2".
    pub size: String,
    /// The class of fit ("6H", "2B"); empty for the standard's default.
    #[serde(default)]
    pub class: String,
    /// A left-hand thread: a modeled one turns the other way.
    #[serde(default)]
    pub left_handed: bool,
}

/// The ISO metric coarse sizes in the order a hole's `metric_index`
/// numbers them.
const INDEXED_METRIC_SIZES: [&str; 10] = [
    "M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12", "M16",
];

/// A thread read as a document or a script gives it: whole, {standard,
/// size, class, left_handed}; as a designation alone, "M6" or "1/4-20",
/// found in the standards that have it; or as a `metric_index`, the place
/// of an ISO metric coarse size in [`INDEXED_METRIC_SIZES`].
impl<'de> Deserialize<'de> for ThreadSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(d)?;
        let shapes = "a thread is a size such as \"M6\" or \"1/4-20\", or {standard, size, \
                      class, left_handed} with standard one of IsoMetricCoarse, IsoMetricFine, \
                      Unc, Unf, Unef, Bsw, Bsf, BspParallel, BspTaper, Npt";
        match &value {
            serde_json::Value::Number(n) => {
                let index = n.as_u64().ok_or_else(|| D::Error::custom(shapes))? as usize;
                Ok(ThreadSpec::new(
                    ThreadStandard::IsoMetricCoarse,
                    &INDEXED_METRIC_SIZES
                        .get(index)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("#{index}")),
                ))
            }
            serde_json::Value::String(size) => ThreadStandard::ALL
                .into_iter()
                .find(|standard| standard.sizes().iter().any(|s| s.name == size))
                .map(|standard| ThreadSpec::new(standard, size))
                .ok_or_else(|| {
                    D::Error::custom(format!("no standard has a thread {size:?}: {shapes}"))
                }),
            serde_json::Value::Object(map) => {
                let standard: ThreadStandard = match map.get("standard") {
                    Some(s) => serde_json::from_value(s.clone())
                        .map_err(|_| D::Error::custom(format!("unknown standard {s}: {shapes}")))?,
                    None => ThreadStandard::default(),
                };
                let size = map
                    .get("size")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| D::Error::custom(format!("the thread has no size: {shapes}")))?;
                Ok(ThreadSpec {
                    standard,
                    size: size.to_string(),
                    class: map
                        .get("class")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    left_handed: map
                        .get("left_handed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
            }
            _ => Err(D::Error::custom(shapes)),
        }
    }
}

/// A thread as its reading above takes it: a designation, or the whole.
impl Describe for ThreadSpec {
    fn shape() -> Shape {
        use core_document::describe::Field;
        let field = |name: &str, shape, default| Field {
            name: name.into(),
            shape,
            default,
            flatten: false,
        };
        Shape::OneOf(vec![
            Shape::Words("a size such as \"M6\" or \"1/4-20\""),
            Shape::Fields(vec![
                field(
                    "standard",
                    ThreadStandard::shape(),
                    core_document::describe::json(&ThreadStandard::default()),
                ),
                field("size", Shape::Text, None),
                field("class", Shape::Text, Some("".into())),
                field("left_handed", Shape::Bool, Some(false.into())),
            ]),
        ])
    }
}

impl ThreadSpec {
    pub fn new(standard: ThreadStandard, size: &str) -> Self {
        Self {
            standard,
            size: size.to_string(),
            class: String::new(),
            left_handed: false,
        }
    }

    /// The class the thread is cut to: its own, or its standard's default.
    pub fn class(&self) -> &str {
        if self.class.is_empty() {
            self.standard.default_class()
        } else {
            &self.class
        }
    }

    /// The thread's size, or why the standard has none by its name or
    /// class.
    pub fn resolve(&self) -> Result<ThreadSize, String> {
        let size = self.standard.size(&self.size).ok_or_else(|| {
            format!(
                "there is no {} size \"{}\"",
                self.standard.label(),
                self.size
            )
        })?;
        let classes = self.standard.classes();
        if !self.class.is_empty() && !classes.contains(&self.class.as_str()) {
            return Err(if classes.is_empty() {
                format!("{} threads have no class to choose", self.standard.label())
            } else {
                format!(
                    "{} has no class {}; it has {}",
                    self.standard.label(),
                    self.class,
                    classes.join(", ")
                )
            });
        }
        Ok(size)
    }

    /// How far the class moves the thread's diameters out, mm.
    pub fn allowance(&self, size: &ThreadSize) -> f64 {
        self.standard.class_allowance(self.class(), size.pitch)
    }

    /// The designation as a drawing gives it: "M6-6H", "1/4-20 UNC-2B",
    /// "G 1/2", "M8x1-6G LH".
    pub fn designation(&self) -> String {
        use ThreadStandard as S;
        let mut text = match self.standard {
            S::IsoMetricCoarse | S::IsoMetricFine => self.size.clone(),
            S::Unc | S::Unf | S::Unef | S::Bsw | S::Bsf => {
                format!("{} {}", self.size, self.standard.label())
            }
            S::BspParallel => format!("G {}", self.size),
            S::BspTaper => format!("Rc {}", self.size),
            S::Npt => format!("{} NPT", self.size),
        };
        let class = self.class();
        if !class.is_empty() {
            text.push('-');
            text.push_str(class);
        }
        if self.left_handed {
            text.push_str(" LH");
        }
        text
    }
}

/// The bottom of a blind hole.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum DrillPoint {
    /// Square to the axis, as an end mill leaves it.
    #[default]
    Flat,
    /// A cone of the drill's included point angle (118° and 135° are the
    /// usual ones; 118° when none is given).
    Angled {
        #[serde(default = "drill_point_angle")]
        #[walk(angle("point_angle", "Drill point angle"))]
        angle_deg: f32,
    },
}

/// A hole's outline across its axis: round, or with a point on the side
/// that faces up as it prints, so a hole lying across the build direction
/// prints its roof as two slopes rather than an overhang.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum HoleShape {
    #[default]
    Round,
    /// A teardrop: the circle and a point toward `up`, whose sides lean
    /// `angle_deg` from it (45° when none is given), tangent to the
    /// circle.
    Teardrop {
        #[serde(default = "overhang_angle")]
        #[walk(angle("teardrop_angle", "Teardrop angle"))]
        angle_deg: f32,
        /// The way the point faces, in the body's frame: its +Z, the
        /// build direction, when none is given.
        #[serde(default = "z_axis")]
        #[walk(setting)]
        up: [f32; 3],
    },
    /// The teardrop with its point cut off flat at the circle's top, so
    /// the roof is a short bridge between the two slopes.
    FlatTop {
        #[serde(default = "overhang_angle")]
        #[walk(angle("flat_top_angle", "Flat top angle"))]
        angle_deg: f32,
        #[serde(default = "z_axis")]
        #[walk(setting)]
        up: [f32; 3],
    },
}

impl HoleShape {
    /// The shapes the panel offers, at their usual angle and facing +Z.
    pub const ALL: [HoleShape; 3] = [
        HoleShape::Round,
        HoleShape::Teardrop {
            angle_deg: 45.0,
            up: [0.0, 0.0, 1.0],
        },
        HoleShape::FlatTop {
            angle_deg: 45.0,
            up: [0.0, 0.0, 1.0],
        },
    ];

    pub fn label(&self) -> &'static str {
        match self {
            HoleShape::Round => "Round",
            HoleShape::Teardrop { .. } => "Teardrop",
            HoleShape::FlatTop { .. } => "Flat top",
        }
    }

    /// The point's lean and the way it faces; `None` for a round hole.
    pub fn point(&self) -> Option<(f32, [f32; 3])> {
        match *self {
            HoleShape::Round => None,
            HoleShape::Teardrop { angle_deg, up } | HoleShape::FlatTop { angle_deg, up } => {
                Some((angle_deg, up))
            }
        }
    }
}

/// A hole's shape as a script may give it: whole, or by its name alone
/// for its usual angle facing +Z.
pub(crate) fn hole_shape_or_name<'de, D>(deserializer: D) -> Result<HoleShape, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Given {
        Shape(HoleShape),
        Name(String),
    }
    match Given::deserialize(deserializer)? {
        Given::Shape(shape) => Ok(shape),
        Given::Name(name) => HoleShape::ALL
            .into_iter()
            .find(|s| shape_name(s) == name)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "no hole shape `{name}`; it is Round, Teardrop or FlatTop"
                ))
            }),
    }
}

/// A shape's name as serde writes its variant.
fn shape_name(shape: &HoleShape) -> &'static str {
    match shape {
        HoleShape::Round => "Round",
        HoleShape::Teardrop { .. } => "Teardrop",
        HoleShape::FlatTop { .. } => "FlatTop",
    }
}

fn overhang_angle() -> f32 {
    45.0
}

pub(super) fn z_axis() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}

pub(super) fn default_thread_pitch() -> f32 {
    1.0
}

pub(super) fn default_thread_depth() -> f32 {
    0.6
}

pub(super) fn default_thread_length() -> f32 {
    10.0
}

fn drill_point_angle() -> f32 {
    118.0
}

fn countersink_angle() -> f32 {
    90.0
}
