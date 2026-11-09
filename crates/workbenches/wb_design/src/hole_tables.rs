//! The tables a hole is sized from: the thread standards and their sizes,
//! the classes an internal thread is cut to, the screw seats per metric
//! size, and the user's own table of cut profiles.
//!
//! Every length here is in millimetres; the inch standards keep their
//! rows in inches, as their standards print them, and convert on the way
//! out.

use serde::{Deserialize, Serialize};

use crate::feature::HoleCut;

/// Millimetres to the inch.
const INCH: f64 = 25.4;

/// The half-angle of a 1:16 taper on the diameter, degrees: the wall of a
/// tapered pipe thread leans in by one part in 32 of its length.
pub fn pipe_taper_deg() -> f64 {
    (1.0f64 / 32.0).atan().to_degrees()
}

/// A thread standard a hole can be sized from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ThreadStandard {
    /// ISO metric, coarse pitch (ISO 261/262).
    #[default]
    IsoMetricCoarse,
    /// ISO metric, fine pitch (ISO 261/262).
    IsoMetricFine,
    /// Unified inch, coarse (ASME B1.1).
    Unc,
    /// Unified inch, fine (ASME B1.1).
    Unf,
    /// Unified inch, extra fine (ASME B1.1).
    Unef,
    /// British Standard Whitworth (BS 84).
    Bsw,
    /// British Standard Fine (BS 84).
    Bsf,
    /// British Standard Pipe, parallel (ISO 228, "G").
    BspParallel,
    /// British Standard Pipe, taper (ISO 7, internal "Rc").
    BspTaper,
    /// American National Pipe Taper (ASME B1.20.1).
    Npt,
}

/// One size of a thread standard, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadSize {
    /// Its designation: "M6", "M8x1", "1/4-20", "1/2".
    pub name: &'static str,
    /// The distance from one thread to the next.
    pub pitch: f64,
    /// The major (nominal) diameter; a tapered thread's where it opens,
    /// at the face.
    pub major: f64,
    /// The basic minor diameter; a tapered thread's at the face.
    pub minor: f64,
    /// The straight drill a tap is run into.
    pub tap_drill: f64,
    /// Clearance hole diameters, close, normal and loose, where the
    /// standard names them.
    pub clearance: Option<[f64; 3]>,
}

impl ThreadStandard {
    pub const ALL: [ThreadStandard; 10] = [
        ThreadStandard::IsoMetricCoarse,
        ThreadStandard::IsoMetricFine,
        ThreadStandard::Unc,
        ThreadStandard::Unf,
        ThreadStandard::Unef,
        ThreadStandard::Bsw,
        ThreadStandard::Bsf,
        ThreadStandard::BspParallel,
        ThreadStandard::BspTaper,
        ThreadStandard::Npt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThreadStandard::IsoMetricCoarse => "ISO metric coarse",
            ThreadStandard::IsoMetricFine => "ISO metric fine",
            ThreadStandard::Unc => "UNC",
            ThreadStandard::Unf => "UNF",
            ThreadStandard::Unef => "UNEF",
            ThreadStandard::Bsw => "BSW",
            ThreadStandard::Bsf => "BSF",
            ThreadStandard::BspParallel => "BSP parallel (G)",
            ThreadStandard::BspTaper => "BSP taper (Rc)",
            ThreadStandard::Npt => "NPT",
        }
    }

    /// The included angle between the thread's flanks, degrees.
    pub fn flank_angle_deg(self) -> f64 {
        match self {
            ThreadStandard::Bsw
            | ThreadStandard::Bsf
            | ThreadStandard::BspParallel
            | ThreadStandard::BspTaper => 55.0,
            _ => 60.0,
        }
    }

    /// Whether the thread narrows into the material, 1:16 on the diameter.
    pub fn is_tapered(self) -> bool {
        matches!(self, ThreadStandard::BspTaper | ThreadStandard::Npt)
    }

    /// Whether the sizes are metric, which the screw seats are made for.
    pub fn is_metric(self) -> bool {
        matches!(
            self,
            ThreadStandard::IsoMetricCoarse | ThreadStandard::IsoMetricFine
        )
    }

    /// The classes of an internal thread of this standard; empty where the
    /// standard has one.
    pub fn classes(self) -> &'static [&'static str] {
        match self {
            ThreadStandard::IsoMetricCoarse | ThreadStandard::IsoMetricFine => {
                &["4H", "5H", "6H", "7H", "8H", "4G", "5G", "6G", "7G", "8G"]
            }
            ThreadStandard::Unc | ThreadStandard::Unf | ThreadStandard::Unef => &["1B", "2B", "3B"],
            ThreadStandard::Bsw | ThreadStandard::Bsf => &["Medium", "Normal"],
            ThreadStandard::BspParallel | ThreadStandard::BspTaper | ThreadStandard::Npt => &[],
        }
    }

    /// The class a thread takes when none is named.
    pub fn default_class(self) -> &'static str {
        match self {
            ThreadStandard::IsoMetricCoarse | ThreadStandard::IsoMetricFine => "6H",
            ThreadStandard::Unc | ThreadStandard::Unf | ThreadStandard::Unef => "2B",
            ThreadStandard::Bsw | ThreadStandard::Bsf => "Medium",
            _ => "",
        }
    }

    /// How far a class moves the internal thread's diameters out, mm: the
    /// fundamental deviation EI of ISO 965-1, (15 + 11 P) µm for position
    /// G and nothing for H. Unified and Whitworth internal threads have no
    /// allowance.
    pub fn class_allowance(self, class: &str, pitch: f64) -> f64 {
        if self.is_metric() && class.ends_with('G') {
            (15.0 + 11.0 * pitch) * 1e-3
        } else {
            0.0
        }
    }

    /// Every size of the standard, smallest first.
    pub fn sizes(self) -> Vec<ThreadSize> {
        match self {
            ThreadStandard::IsoMetricCoarse => METRIC_COARSE.iter().map(metric_size).collect(),
            ThreadStandard::IsoMetricFine => METRIC_FINE.iter().map(metric_size).collect(),
            ThreadStandard::Unc => UNC.iter().map(unified_size).collect(),
            ThreadStandard::Unf => UNF.iter().map(unified_size).collect(),
            ThreadStandard::Unef => UNEF.iter().map(unified_size).collect(),
            ThreadStandard::Bsw => BSW.iter().map(whitworth_size).collect(),
            ThreadStandard::Bsf => BSF.iter().map(whitworth_size).collect(),
            ThreadStandard::BspParallel => BSP
                .iter()
                .map(|&(name, tpi, major, parallel_drill, _)| {
                    whitworth_size(&(name, tpi, major / INCH, parallel_drill))
                })
                .collect(),
            ThreadStandard::BspTaper => BSP
                .iter()
                .filter(|row| BSP_TAPER.contains(&row.0))
                .map(|&(name, tpi, major, _, taper_drill)| {
                    whitworth_size(&(name, tpi, major / INCH, taper_drill))
                })
                .collect(),
            ThreadStandard::Npt => NPT.iter().map(npt_size).collect(),
        }
    }

    /// The size called `name`.
    pub fn size(self, name: &str) -> Option<ThreadSize> {
        self.sizes().into_iter().find(|s| s.name == name)
    }
}

/// ISO metric, coarse: designation, pitch, major, tap drill.
const METRIC_COARSE: &[(&str, f64, f64, f64)] = &[
    ("M1.6", 0.35, 1.6, 1.25),
    ("M2", 0.4, 2.0, 1.6),
    ("M2.5", 0.45, 2.5, 2.05),
    ("M3", 0.5, 3.0, 2.5),
    ("M3.5", 0.6, 3.5, 2.9),
    ("M4", 0.7, 4.0, 3.3),
    ("M5", 0.8, 5.0, 4.2),
    ("M6", 1.0, 6.0, 5.0),
    ("M8", 1.25, 8.0, 6.8),
    ("M10", 1.5, 10.0, 8.5),
    ("M12", 1.75, 12.0, 10.2),
    ("M14", 2.0, 14.0, 12.0),
    ("M16", 2.0, 16.0, 14.0),
    ("M18", 2.5, 18.0, 15.5),
    ("M20", 2.5, 20.0, 17.5),
    ("M22", 2.5, 22.0, 19.5),
    ("M24", 3.0, 24.0, 21.0),
    ("M27", 3.0, 27.0, 24.0),
    ("M30", 3.5, 30.0, 26.5),
    ("M33", 3.5, 33.0, 29.5),
    ("M36", 4.0, 36.0, 32.0),
    ("M42", 4.5, 42.0, 37.5),
    ("M48", 5.0, 48.0, 43.0),
];

/// ISO metric, fine: designation, pitch, major, tap drill.
const METRIC_FINE: &[(&str, f64, f64, f64)] = &[
    ("M3x0.35", 0.35, 3.0, 2.65),
    ("M4x0.5", 0.5, 4.0, 3.5),
    ("M5x0.5", 0.5, 5.0, 4.5),
    ("M6x0.75", 0.75, 6.0, 5.2),
    ("M8x0.75", 0.75, 8.0, 7.2),
    ("M8x1", 1.0, 8.0, 7.0),
    ("M10x0.75", 0.75, 10.0, 9.2),
    ("M10x1", 1.0, 10.0, 9.0),
    ("M10x1.25", 1.25, 10.0, 8.8),
    ("M12x1", 1.0, 12.0, 11.0),
    ("M12x1.25", 1.25, 12.0, 10.8),
    ("M12x1.5", 1.5, 12.0, 10.5),
    ("M14x1.5", 1.5, 14.0, 12.5),
    ("M16x1", 1.0, 16.0, 15.0),
    ("M16x1.5", 1.5, 16.0, 14.5),
    ("M18x1.5", 1.5, 18.0, 16.5),
    ("M20x1.5", 1.5, 20.0, 18.5),
    ("M20x2", 2.0, 20.0, 18.0),
    ("M22x1.5", 1.5, 22.0, 20.5),
    ("M24x1.5", 1.5, 24.0, 22.5),
    ("M24x2", 2.0, 24.0, 22.0),
    ("M27x2", 2.0, 27.0, 25.0),
    ("M30x2", 2.0, 30.0, 28.0),
    ("M36x3", 3.0, 36.0, 33.0),
];

/// ISO 273 clearance holes by nominal diameter: fine (close), medium
/// (normal) and coarse (loose).
const ISO_273: &[(f64, [f64; 3])] = &[
    (1.6, [1.7, 1.8, 2.0]),
    (2.0, [2.2, 2.4, 2.6]),
    (2.5, [2.7, 2.9, 3.1]),
    (3.0, [3.2, 3.4, 3.6]),
    (3.5, [3.7, 3.9, 4.2]),
    (4.0, [4.3, 4.5, 4.8]),
    (5.0, [5.3, 5.5, 5.8]),
    (6.0, [6.4, 6.6, 7.0]),
    (8.0, [8.4, 9.0, 10.0]),
    (10.0, [10.5, 11.0, 12.0]),
    (12.0, [13.0, 13.5, 14.5]),
    (14.0, [15.0, 15.5, 16.5]),
    (16.0, [17.0, 17.5, 18.5]),
    (18.0, [19.0, 20.0, 21.0]),
    (20.0, [21.0, 22.0, 24.0]),
    (22.0, [23.0, 24.0, 26.0]),
    (24.0, [25.0, 26.0, 28.0]),
    (27.0, [28.0, 30.0, 32.0]),
    (30.0, [31.0, 33.0, 35.0]),
    (33.0, [34.0, 36.0, 38.0]),
    (36.0, [37.0, 39.0, 42.0]),
    (42.0, [43.0, 45.0, 48.0]),
    (48.0, [50.0, 52.0, 56.0]),
];

fn metric_clearance(nominal: f64) -> Option<[f64; 3]> {
    ISO_273
        .iter()
        .find(|(d, _)| (d - nominal).abs() < 1e-9)
        .map(|(_, c)| *c)
}

fn metric_size(&(name, pitch, major, tap_drill): &(&'static str, f64, f64, f64)) -> ThreadSize {
    ThreadSize {
        name,
        pitch,
        major,
        // D1 = D - 2 · 5/8 · H, H = √3/2 · P.
        minor: major - 1.082_532 * pitch,
        tap_drill,
        clearance: metric_clearance(major),
    }
}

/// Unified inch: designation, threads per inch, major (in), tap drill for
/// a 75 % thread (in).
const UNC: &[(&str, f64, f64, f64)] = &[
    ("#1-64", 64.0, 0.0730, 0.0595),
    ("#2-56", 56.0, 0.0860, 0.0700),
    ("#3-48", 48.0, 0.0990, 0.0785),
    ("#4-40", 40.0, 0.1120, 0.0890),
    ("#5-40", 40.0, 0.1250, 0.1015),
    ("#6-32", 32.0, 0.1380, 0.1065),
    ("#8-32", 32.0, 0.1640, 0.1360),
    ("#10-24", 24.0, 0.1900, 0.1495),
    ("#12-24", 24.0, 0.2160, 0.1770),
    ("1/4-20", 20.0, 0.2500, 0.2010),
    ("5/16-18", 18.0, 0.3125, 0.2570),
    ("3/8-16", 16.0, 0.3750, 0.3125),
    ("7/16-14", 14.0, 0.4375, 0.3680),
    ("1/2-13", 13.0, 0.5000, 0.4219),
    ("9/16-12", 12.0, 0.5625, 0.4844),
    ("5/8-11", 11.0, 0.6250, 0.5312),
    ("3/4-10", 10.0, 0.7500, 0.6562),
    ("7/8-9", 9.0, 0.8750, 0.7656),
    ("1-8", 8.0, 1.0000, 0.8750),
];

const UNF: &[(&str, f64, f64, f64)] = &[
    ("#0-80", 80.0, 0.0600, 0.0469),
    ("#1-72", 72.0, 0.0730, 0.0595),
    ("#2-64", 64.0, 0.0860, 0.0700),
    ("#3-56", 56.0, 0.0990, 0.0820),
    ("#4-48", 48.0, 0.1120, 0.0935),
    ("#5-44", 44.0, 0.1250, 0.1040),
    ("#6-40", 40.0, 0.1380, 0.1130),
    ("#8-36", 36.0, 0.1640, 0.1360),
    ("#10-32", 32.0, 0.1900, 0.1590),
    ("#12-28", 28.0, 0.2160, 0.1820),
    ("1/4-28", 28.0, 0.2500, 0.2130),
    ("5/16-24", 24.0, 0.3125, 0.2720),
    ("3/8-24", 24.0, 0.3750, 0.3320),
    ("7/16-20", 20.0, 0.4375, 0.3906),
    ("1/2-20", 20.0, 0.5000, 0.4531),
    ("9/16-18", 18.0, 0.5625, 0.5156),
    ("5/8-18", 18.0, 0.6250, 0.5781),
    ("3/4-16", 16.0, 0.7500, 0.6875),
    ("7/8-14", 14.0, 0.8750, 0.8125),
    ("1-12", 12.0, 1.0000, 0.9219),
];

const UNEF: &[(&str, f64, f64, f64)] = &[
    ("#12-32", 32.0, 0.2160, 0.1850),
    ("1/4-32", 32.0, 0.2500, 0.2188),
    ("5/16-32", 32.0, 0.3125, 0.2812),
    ("3/8-32", 32.0, 0.3750, 0.3438),
    ("7/16-28", 28.0, 0.4375, 0.4062),
    ("1/2-28", 28.0, 0.5000, 0.4688),
    ("9/16-24", 24.0, 0.5625, 0.5156),
    ("5/8-24", 24.0, 0.6250, 0.5781),
    ("11/16-24", 24.0, 0.6875, 0.6406),
    ("3/4-20", 20.0, 0.7500, 0.7031),
    ("13/16-20", 20.0, 0.8125, 0.7656),
    ("7/8-20", 20.0, 0.8750, 0.8281),
    ("1-20", 20.0, 1.0000, 0.9531),
];

/// Clearance holes for inch screws after ASME B18.2.8, by major diameter
/// (in): close, normal and loose (in).
const INCH_CLEARANCE: &[(f64, [f64; 3])] = &[
    (0.0600, [0.0635, 0.0700, 0.0760]),
    (0.0730, [0.0760, 0.0810, 0.0860]),
    (0.0860, [0.0890, 0.0960, 0.1040]),
    (0.0990, [0.1040, 0.1100, 0.1160]),
    (0.1120, [0.1160, 0.1200, 0.1285]),
    (0.1250, [0.1285, 0.1360, 0.1495]),
    (0.1380, [0.1440, 0.1495, 0.1610]),
    (0.1640, [0.1695, 0.1770, 0.1910]),
    (0.1900, [0.1960, 0.2010, 0.2210]),
    (0.2160, [0.2210, 0.2280, 0.2500]),
    (0.2500, [0.2570, 0.2660, 0.2810]),
    (0.3125, [0.3230, 0.3320, 0.3440]),
    (0.3750, [0.3860, 0.3970, 0.4060]),
    (0.4375, [0.4530, 0.4690, 0.4840]),
    (0.5000, [0.5160, 0.5310, 0.5620]),
    (0.5625, [0.5780, 0.5940, 0.6250]),
    (0.6250, [0.6410, 0.6560, 0.6880]),
    (0.6875, [0.7030, 0.7190, 0.7500]),
    (0.7500, [0.7660, 0.7810, 0.8120]),
    (0.8125, [0.8280, 0.8440, 0.8750]),
    (0.8750, [0.8910, 0.9060, 0.9380]),
    (1.0000, [1.0160, 1.0310, 1.0620]),
];

fn unified_size(&(name, tpi, major, tap): &(&'static str, f64, f64, f64)) -> ThreadSize {
    let pitch = INCH / tpi;
    ThreadSize {
        name,
        pitch,
        major: major * INCH,
        minor: major * INCH - 1.082_532 * pitch,
        tap_drill: tap * INCH,
        clearance: INCH_CLEARANCE
            .iter()
            .find(|(d, _)| (d - major).abs() < 1e-6)
            .map(|(_, c)| c.map(|v| v * INCH)),
    }
}

/// British Standard Whitworth and Fine: designation, threads per inch,
/// major (in), tap drill (mm).
const BSW: &[(&str, f64, f64, f64)] = &[
    ("1/8", 40.0, 0.1250, 2.55),
    ("3/16", 24.0, 0.1875, 3.7),
    ("1/4", 20.0, 0.2500, 5.1),
    ("5/16", 18.0, 0.3125, 6.5),
    ("3/8", 16.0, 0.3750, 7.9),
    ("7/16", 14.0, 0.4375, 9.3),
    ("1/2", 12.0, 0.5000, 10.5),
    ("5/8", 11.0, 0.6250, 13.5),
    ("3/4", 10.0, 0.7500, 16.25),
    ("7/8", 9.0, 0.8750, 19.25),
    ("1", 8.0, 1.0000, 22.0),
];

const BSF: &[(&str, f64, f64, f64)] = &[
    ("3/16", 32.0, 0.1875, 4.0),
    ("7/32", 28.0, 0.2188, 4.6),
    ("1/4", 26.0, 0.2500, 5.3),
    ("9/32", 26.0, 0.2812, 6.1),
    ("5/16", 22.0, 0.3125, 6.8),
    ("3/8", 20.0, 0.3750, 8.3),
    ("7/16", 18.0, 0.4375, 9.7),
    ("1/2", 16.0, 0.5000, 11.1),
    ("9/16", 16.0, 0.5625, 12.7),
    ("5/8", 14.0, 0.6250, 14.0),
    ("3/4", 12.0, 0.7500, 16.75),
    ("7/8", 11.0, 0.8750, 19.75),
    ("1", 10.0, 1.0000, 22.75),
];

/// The depth of a Whitworth form, crest to root: 0.640327 P.
const WHITWORTH_DEPTH: f64 = 0.640_327;

fn whitworth_size(&(name, tpi, major, tap_drill): &(&'static str, f64, f64, f64)) -> ThreadSize {
    let pitch = INCH / tpi;
    ThreadSize {
        name,
        pitch,
        major: major * INCH,
        minor: major * INCH - 2.0 * WHITWORTH_DEPTH * pitch,
        tap_drill,
        clearance: None,
    }
}

/// British Standard Pipe (ISO 228 and ISO 7 share the form and the gauge
/// diameters): designation, threads per inch, major (mm, a taper
/// thread's at its gauge plane, which is an internal thread's face), tap
/// drill for the parallel thread and for the taper (mm).
const BSP: &[(&str, f64, f64, f64, f64)] = &[
    ("1/16", 28.0, 7.723, 6.8, 6.6),
    ("1/8", 28.0, 9.728, 8.8, 8.4),
    ("1/4", 19.0, 13.157, 11.8, 11.2),
    ("3/8", 19.0, 16.662, 15.25, 14.75),
    ("1/2", 14.0, 20.955, 19.0, 18.25),
    ("5/8", 14.0, 22.911, 21.0, 20.25),
    ("3/4", 14.0, 26.441, 24.5, 23.75),
    ("7/8", 14.0, 30.201, 28.25, 27.5),
    ("1", 11.0, 33.249, 30.75, 30.0),
    ("1-1/4", 11.0, 41.910, 39.5, 38.5),
    ("1-1/2", 11.0, 47.803, 45.25, 44.5),
    ("2", 11.0, 59.614, 57.0, 56.0),
];

/// The BSP sizes ISO 7 gives as taper threads.
const BSP_TAPER: &[&str] = &[
    "1/16", "1/8", "1/4", "3/8", "1/2", "3/4", "1", "1-1/4", "1-1/2", "2",
];

/// NPT: designation, threads per inch, pitch diameter at the small end
/// E0 (in), hand-tight engagement L1 (in), tap drill (in).
const NPT: &[(&str, f64, f64, f64, f64)] = &[
    ("1/16", 27.0, 0.27118, 0.160, 0.2500),
    ("1/8", 27.0, 0.36351, 0.1615, 0.3320),
    ("1/4", 18.0, 0.47739, 0.2278, 0.4375),
    ("3/8", 18.0, 0.61201, 0.240, 0.5781),
    ("1/2", 14.0, 0.75843, 0.320, 0.7188),
    ("3/4", 14.0, 0.96768, 0.339, 0.9219),
    ("1", 11.5, 1.21363, 0.400, 1.1562),
    ("1-1/4", 11.5, 1.55713, 0.420, 1.5000),
    ("1-1/2", 11.5, 1.79609, 0.420, 1.7344),
    ("2", 11.5, 2.26902, 0.436, 2.2188),
];

/// An internal NPT thread opens at the L1 gauge plane: its pitch diameter
/// there is E1 = E0 + L1 / 16, the thread 0.8 P deep about it.
fn npt_size(&(name, tpi, e0, l1, tap): &(&'static str, f64, f64, f64, f64)) -> ThreadSize {
    let pitch = INCH / tpi;
    let e1 = (e0 + l1 / 16.0) * INCH;
    ThreadSize {
        name,
        pitch,
        major: e1 + 0.8 * pitch,
        minor: e1 - 0.8 * pitch,
        tap_drill: tap * INCH,
        clearance: None,
    }
}

/// The seat a screw's head sits in, sized from the hole's metric size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScrewSeat {
    /// A counterbore for an ISO 4762 socket head cap screw, DIN 974-1
    /// diameter, as deep as the head and a little more.
    #[default]
    SocketHead,
    /// A 90° countersink for an ISO 10642 countersunk socket screw, as
    /// wide as the head and a little more.
    Countersunk,
    /// A counterbore for an ISO 7380 button head screw.
    ButtonHead,
    /// A 90° countersink for an ISO 2009 slotted countersunk screw.
    SlottedCountersunk,
    /// A 90° countersink for an ISO 7046 cross recessed countersunk screw,
    /// whose head is the slotted one's.
    CrossCountersunk,
    /// A counterbore for a DIN 7984 low head cap screw.
    LowHeadCap,
    /// A counterbore for an ISO 4762 cap screw on an ISO 7089 washer, DIN
    /// 974-1's wider row.
    CapScrewWithWasher,
    /// A counterbore for an ISO 4017 hex head screw, room for a socket
    /// wrench round the head (DIN 974-2).
    HexHead,
}

impl ScrewSeat {
    pub const ALL: [ScrewSeat; 8] = [
        ScrewSeat::SocketHead,
        ScrewSeat::Countersunk,
        ScrewSeat::ButtonHead,
        ScrewSeat::SlottedCountersunk,
        ScrewSeat::CrossCountersunk,
        ScrewSeat::LowHeadCap,
        ScrewSeat::CapScrewWithWasher,
        ScrewSeat::HexHead,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ScrewSeat::SocketHead => "ISO 4762 seat",
            ScrewSeat::Countersunk => "ISO 10642 seat",
            ScrewSeat::ButtonHead => "ISO 7380 seat",
            ScrewSeat::SlottedCountersunk => "ISO 2009 seat",
            ScrewSeat::CrossCountersunk => "ISO 7046 seat",
            ScrewSeat::LowHeadCap => "DIN 7984 seat",
            ScrewSeat::CapScrewWithWasher => "ISO 4762 + washer seat",
            ScrewSeat::HexHead => "ISO 4017 seat",
        }
    }

    /// Whether the seat is a countersink rather than a counterbore.
    fn sunk(self) -> bool {
        matches!(
            self,
            ScrewSeat::Countersunk | ScrewSeat::SlottedCountersunk | ScrewSeat::CrossCountersunk
        )
    }

    /// The cut this seat makes for a screw of `nominal` diameter.
    pub fn cut(self, nominal: f64) -> Option<HoleCut> {
        let table = match self {
            ScrewSeat::SocketHead => SOCKET_HEAD_SEATS,
            ScrewSeat::Countersunk => COUNTERSUNK_SEATS,
            ScrewSeat::ButtonHead => BUTTON_HEAD_SEATS,
            ScrewSeat::SlottedCountersunk | ScrewSeat::CrossCountersunk => {
                SLOTTED_COUNTERSUNK_SEATS
            }
            ScrewSeat::LowHeadCap => LOW_HEAD_SEATS,
            ScrewSeat::CapScrewWithWasher => WASHER_SEATS,
            ScrewSeat::HexHead => HEX_HEAD_SEATS,
        };
        let &(_, diameter, depth) = table.iter().find(|(d, ..)| (d - nominal).abs() < 1e-9)?;
        Some(if self.sunk() {
            HoleCut::Countersink {
                diameter: diameter as f32,
                angle_deg: 90.0,
            }
        } else {
            HoleCut::Counterbore {
                diameter: diameter as f32,
                depth: depth as f32,
            }
        })
    }
}

/// ISO 4762 counterbores: nominal, DIN 974-1 diameter, depth (the head
/// height k = d and a clearance).
const SOCKET_HEAD_SEATS: &[(f64, f64, f64)] = &[
    (1.6, 3.5, 1.9),
    (2.0, 4.4, 2.4),
    (2.5, 5.5, 2.9),
    (3.0, 6.5, 3.4),
    (4.0, 8.0, 4.4),
    (5.0, 10.0, 5.4),
    (6.0, 11.0, 6.4),
    (8.0, 15.0, 8.6),
    (10.0, 18.0, 10.6),
    (12.0, 20.0, 12.6),
    (14.0, 24.0, 14.6),
    (16.0, 26.0, 16.6),
    (20.0, 33.0, 20.6),
    (24.0, 40.0, 24.8),
    (27.0, 46.0, 27.8),
    (30.0, 50.0, 30.8),
    (36.0, 58.0, 36.8),
];

/// ISO 10642 countersinks: nominal, diameter at the face (the head's
/// theoretical dk and a clearance), unused depth.
const COUNTERSUNK_SEATS: &[(f64, f64, f64)] = &[
    (3.0, 6.9, 0.0),
    (4.0, 9.2, 0.0),
    (5.0, 11.5, 0.0),
    (6.0, 13.7, 0.0),
    (8.0, 18.3, 0.0),
    (10.0, 22.7, 0.0),
    (12.0, 27.2, 0.0),
    (14.0, 31.1, 0.0),
    (16.0, 33.9, 0.0),
    (20.0, 40.7, 0.0),
];

/// ISO 7380 button heads: nominal, counterbore diameter (the head's dk and
/// a clearance), depth (its height k and a clearance).
const BUTTON_HEAD_SEATS: &[(f64, f64, f64)] = &[
    (3.0, 6.5, 2.1),
    (4.0, 8.5, 2.6),
    (5.0, 10.5, 3.2),
    (6.0, 11.5, 3.7),
    (8.0, 15.0, 4.8),
    (10.0, 18.5, 5.9),
    (12.0, 22.0, 7.0),
    (16.0, 29.0, 9.2),
];

/// ISO 2009 (and ISO 7046) countersunk heads: nominal, diameter at the
/// face (dk and a clearance), unused depth.
const SLOTTED_COUNTERSUNK_SEATS: &[(f64, f64, f64)] = &[
    (1.6, 4.0, 0.0),
    (2.0, 4.8, 0.0),
    (2.5, 5.9, 0.0),
    (3.0, 6.7, 0.0),
    (4.0, 9.8, 0.0),
    (5.0, 10.8, 0.0),
    (6.0, 13.0, 0.0),
    (8.0, 17.7, 0.0),
    (10.0, 20.4, 0.0),
];

/// DIN 7984 low head cap screws: nominal, counterbore diameter (DIN 974-1),
/// depth (head height k and a clearance).
const LOW_HEAD_SEATS: &[(f64, f64, f64)] = &[
    (3.0, 6.5, 2.4),
    (4.0, 8.0, 3.2),
    (5.0, 10.0, 3.9),
    (6.0, 11.0, 4.4),
    (8.0, 15.0, 5.4),
    (10.0, 18.0, 6.4),
    (12.0, 20.0, 7.4),
    (16.0, 26.0, 9.4),
    (20.0, 33.0, 11.4),
];

/// ISO 4762 cap screws on ISO 7089 washers: nominal, counterbore diameter
/// (DIN 974-1's washer row), depth (head height, washer and a clearance).
const WASHER_SEATS: &[(f64, f64, f64)] = &[
    (3.0, 8.0, 3.9),
    (4.0, 10.0, 5.2),
    (5.0, 11.0, 6.4),
    (6.0, 13.0, 8.0),
    (8.0, 18.0, 10.0),
    (10.0, 22.0, 12.4),
    (12.0, 26.0, 14.9),
    (16.0, 33.0, 19.4),
    (20.0, 40.0, 23.4),
];

/// ISO 4017 hex heads: nominal, counterbore diameter (DIN 974-2, a socket
/// wrench's room), depth (head height k and a clearance).
const HEX_HEAD_SEATS: &[(f64, f64, f64)] = &[
    (3.0, 11.0, 2.4),
    (4.0, 13.0, 3.2),
    (5.0, 15.0, 3.9),
    (6.0, 18.0, 4.4),
    (8.0, 24.0, 5.7),
    (10.0, 28.0, 6.8),
    (12.0, 33.0, 7.9),
    (16.0, 40.0, 10.4),
    (20.0, 46.0, 12.9),
];

/// A hex nut standard a nut trap is sized from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NutStandard {
    /// ISO 4032 hex nuts, style 1.
    #[default]
    Iso4032,
    /// DIN 934 hex nuts, wider across the flats from M10 to M14.
    Din934,
}

/// A nut's size: across its flats and how thick it is, mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NutSize {
    pub across_flats: f64,
    pub thickness: f64,
}

impl NutStandard {
    pub const ALL: [NutStandard; 2] = [NutStandard::Iso4032, NutStandard::Din934];

    pub fn label(self) -> &'static str {
        match self {
            NutStandard::Iso4032 => "ISO 4032",
            NutStandard::Din934 => "DIN 934",
        }
    }

    /// The nut for a screw of `nominal` diameter, mm.
    pub fn nut(self, nominal: f64) -> Option<NutSize> {
        let table = match self {
            NutStandard::Iso4032 => ISO_4032_NUTS,
            NutStandard::Din934 => DIN_934_NUTS,
        };
        table.iter().find(|(d, ..)| (d - nominal).abs() < 1e-9).map(
            |&(_, across_flats, thickness)| NutSize {
                across_flats,
                thickness,
            },
        )
    }
}

/// ISO 4032 hex nuts: nominal, across flats s, thickness m (its largest).
const ISO_4032_NUTS: &[(f64, f64, f64)] = &[
    (1.6, 3.2, 1.3),
    (2.0, 4.0, 1.6),
    (2.5, 5.0, 2.0),
    (3.0, 5.5, 2.4),
    (3.5, 6.0, 2.8),
    (4.0, 7.0, 3.2),
    (5.0, 8.0, 4.7),
    (6.0, 10.0, 5.2),
    (8.0, 13.0, 6.8),
    (10.0, 16.0, 8.4),
    (12.0, 18.0, 10.8),
    (14.0, 21.0, 12.8),
    (16.0, 24.0, 14.8),
    (20.0, 30.0, 18.0),
    (24.0, 36.0, 21.5),
    (30.0, 46.0, 25.6),
    (36.0, 55.0, 31.0),
];

/// DIN 934 hex nuts: nominal, across flats s, thickness m.
const DIN_934_NUTS: &[(f64, f64, f64)] = &[
    (1.6, 3.2, 1.3),
    (2.0, 4.0, 1.6),
    (2.5, 5.0, 2.0),
    (3.0, 5.5, 2.4),
    (3.5, 6.0, 2.8),
    (4.0, 7.0, 3.2),
    (5.0, 8.0, 4.0),
    (6.0, 10.0, 5.0),
    (8.0, 13.0, 6.5),
    (10.0, 17.0, 8.0),
    (12.0, 19.0, 10.0),
    (14.0, 22.0, 11.0),
    (16.0, 24.0, 13.0),
    (20.0, 30.0, 16.0),
    (24.0, 36.0, 19.0),
    (30.0, 46.0, 24.0),
    (36.0, 55.0, 29.0),
];

/// A named cut from the user's table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutProfile {
    pub name: String,
    pub cut: HoleCut,
}

/// The user's table of cut profiles, as its file holds it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CutProfileTable {
    #[serde(default)]
    pub profiles: Vec<CutProfile>,
}

/// The file the user's cut profiles are read from, in the application's
/// configuration folder.
pub const CUT_PROFILES_FILE: &str = "hole_cuts.json";

/// Read a table of cut profiles; the profiles a hole cannot use (a screw
/// seat, which needs the hole's size, or no cut at all) are left out.
pub fn parse_cut_profiles(text: &str) -> Result<Vec<CutProfile>, String> {
    let table: CutProfileTable = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(table
        .profiles
        .into_iter()
        .filter(|p| !matches!(p.cut, HoleCut::None | HoleCut::Seat { .. }))
        .collect())
}

static USER_CUT_PROFILES: std::sync::OnceLock<Vec<CutProfile>> = std::sync::OnceLock::new();

/// The user's cut profiles, read from [`CUT_PROFILES_FILE`] the first time
/// they are asked for; none when there is no file or it does not read.
pub fn user_cut_profiles() -> &'static [CutProfile] {
    USER_CUT_PROFILES.get_or_init(|| {
        let Some(path) = settings::config_path(CUT_PROFILES_FILE) else {
            return Vec::new();
        };
        settings::json_file::load::<CutProfileTable>(&path)
            .map(|table| {
                table
                    .profiles
                    .into_iter()
                    .filter(|p| !matches!(p.cut, HoleCut::None | HoleCut::Seat { .. }))
                    .collect()
            })
            .unwrap_or_else(|e| {
                tracing::warn!("{e}");
                Vec::new()
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_standard_lists_sane_sizes() {
        for standard in ThreadStandard::ALL {
            let sizes = standard.sizes();
            assert!(!sizes.is_empty(), "{standard:?}");
            let mut names: Vec<&str> = sizes.iter().map(|s| s.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), sizes.len(), "{standard:?}: names repeat");
            for pair in sizes.windows(2) {
                assert!(
                    pair[0].major <= pair[1].major,
                    "{standard:?}: {} before {}",
                    pair[0].name,
                    pair[1].name
                );
            }
            for size in &sizes {
                let what = format!("{standard:?} {}", size.name);
                assert!(size.pitch > 0.0 && size.pitch < 0.25 * size.major, "{what}");
                assert!(size.minor > 0.0 && size.minor < size.major, "{what}");
                // A tap drill sits near the minor diameter, well inside the
                // major.
                assert!(
                    size.tap_drill > 0.85 * size.minor && size.tap_drill < size.major,
                    "{what}: tap {} minor {} major {}",
                    size.tap_drill,
                    size.minor,
                    size.major
                );
                if let Some([close, normal, loose]) = size.clearance {
                    assert!(
                        size.major < close && close <= normal && normal <= loose,
                        "{what}: {close} {normal} {loose}"
                    );
                }
                assert_eq!(standard.size(size.name), Some(*size));
            }
            let classes = standard.classes();
            assert_eq!(
                classes.is_empty(),
                standard.default_class().is_empty(),
                "{standard:?}"
            );
            if !classes.is_empty() {
                assert!(classes.contains(&standard.default_class()));
            }
        }
    }

    #[test]
    fn metric_sizes_carry_iso_273_clearances() {
        let m6 = ThreadStandard::IsoMetricCoarse.size("M6").unwrap();
        assert_eq!((m6.pitch, m6.tap_drill), (1.0, 5.0));
        assert_eq!(m6.clearance, Some([6.4, 6.6, 7.0]));
        let fine = ThreadStandard::IsoMetricFine.size("M8x1").unwrap();
        assert_eq!(fine.clearance, Some([8.4, 9.0, 10.0]));
        let unc = ThreadStandard::Unc.size("1/4-20").unwrap();
        assert!((unc.pitch - 1.27).abs() < 1e-9 && (unc.major - 6.35).abs() < 1e-9);
        assert!((unc.tap_drill - 5.1054).abs() < 1e-4, "#7 drill");
        assert!(unc.clearance.is_some());
        assert!(ThreadStandard::Bsw.size("1/4").unwrap().clearance.is_none());
    }

    #[test]
    fn pipe_threads_open_at_their_gauge_diameter() {
        // 1/4 NPT: E1 = 0.47739 + 0.2278/16 in, 18 threads per inch.
        let npt = ThreadStandard::Npt.size("1/4").unwrap();
        let e1 = (0.47739 + 0.2278 / 16.0) * 25.4;
        assert!((npt.major - (e1 + 0.8 * 25.4 / 18.0)).abs() < 1e-9);
        assert!((npt.minor - (e1 - 0.8 * 25.4 / 18.0)).abs() < 1e-9);
        let rc = ThreadStandard::BspTaper.size("1/2").unwrap();
        assert!((rc.major - 20.955).abs() < 1e-9 && (rc.pitch - 25.4 / 14.0).abs() < 1e-9);
        assert!(
            ThreadStandard::BspTaper.size("5/8").is_none(),
            "ISO 7 has no 5/8"
        );
        assert!(ThreadStandard::BspParallel.size("5/8").is_some());
        assert!((pipe_taper_deg() - 1.789_910_608).abs() < 1e-6);
        assert!(ThreadStandard::Npt.is_tapered() && !ThreadStandard::BspParallel.is_tapered());
    }

    #[test]
    fn a_g_class_moves_a_metric_thread_out() {
        let s = ThreadStandard::IsoMetricCoarse;
        assert_eq!(s.class_allowance("6H", 1.0), 0.0);
        assert!((s.class_allowance("6G", 1.0) - 0.026).abs() < 1e-12);
        assert_eq!(ThreadStandard::Unc.class_allowance("2B", 1.27), 0.0);
    }

    #[test]
    fn seats_come_from_their_tables() {
        assert_eq!(
            ScrewSeat::SocketHead.cut(6.0),
            Some(HoleCut::Counterbore {
                diameter: 11.0,
                depth: 6.4
            })
        );
        assert_eq!(
            ScrewSeat::Countersunk.cut(8.0),
            Some(HoleCut::Countersink {
                diameter: 18.3,
                angle_deg: 90.0
            })
        );
        assert_eq!(ScrewSeat::Countersunk.cut(2.0), None);
        // Every seat is wider than the loosest clearance hole it sits on.
        for seat in ScrewSeat::ALL {
            for size in ThreadStandard::IsoMetricCoarse.sizes() {
                let Some(cut) = seat.cut(size.major) else {
                    continue;
                };
                let wide = match cut {
                    HoleCut::Counterbore { diameter, .. }
                    | HoleCut::Countersink { diameter, .. } => f64::from(diameter),
                    _ => unreachable!(),
                };
                assert!(wide > size.clearance.unwrap()[2], "{seat:?} {}", size.name);
            }
        }
    }

    #[test]
    fn cut_profiles_read_from_their_file() {
        let text = r#"{
            "profiles": [
                { "name": "M3 heat insert", "cut": { "Counterbore": { "diameter": 4.2, "depth": 5.0 } } },
                { "name": "Deburr", "cut": { "Countersink": { "diameter": 6.0, "angle_deg": 90.0 } } },
                { "name": "Seat", "cut": { "Seat": { "seat": "SocketHead" } } }
            ]
        }"#;
        let profiles = parse_cut_profiles(text).unwrap();
        assert_eq!(profiles.len(), 2, "a seat needs a size and is left out");
        assert_eq!(profiles[0].name, "M3 heat insert");
        assert_eq!(
            profiles[0].cut,
            HoleCut::Counterbore {
                diameter: 4.2,
                depth: 5.0
            }
        );
        assert!(parse_cut_profiles("{ nope").is_err());
        assert_eq!(parse_cut_profiles("{}").unwrap(), Vec::new());
    }

    #[test]
    fn every_seat_cuts_the_way_its_head_sits() {
        for seat in ScrewSeat::ALL {
            let cut = seat.cut(6.0).unwrap_or_else(|| panic!("{seat:?} has M6"));
            match cut {
                HoleCut::Countersink {
                    diameter,
                    angle_deg,
                } => {
                    assert!(seat.sunk(), "{seat:?}");
                    assert!(diameter > 6.0 && angle_deg == 90.0);
                }
                HoleCut::Counterbore { diameter, depth } => {
                    assert!(!seat.sunk(), "{seat:?}");
                    assert!(diameter > 6.0 && depth > 0.0, "{seat:?}");
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            ScrewSeat::ButtonHead.cut(5.0),
            Some(HoleCut::Counterbore {
                diameter: 10.5,
                depth: 3.2
            })
        );
    }

    /// The nut tables differ only where DIN 934 is wider across the flats
    /// (M10 to M14) or thinner; every nut is half again its screw across
    /// the flats or more, and its pocket is the nut and the clearance.
    #[test]
    fn nuts_are_sized_by_their_standard() {
        let iso = NutStandard::Iso4032.nut(10.0).unwrap();
        let din = NutStandard::Din934.nut(10.0).unwrap();
        assert_eq!((iso.across_flats, din.across_flats), (16.0, 17.0));
        for (nominal, ..) in ISO_4032_NUTS.iter().chain(DIN_934_NUTS) {
            for standard in NutStandard::ALL {
                let nut = standard.nut(*nominal).unwrap();
                assert!(nut.across_flats >= 1.5 * nominal, "M{nominal}");
                assert!(nut.thickness > 0.5 * nominal && nut.thickness < *nominal);
            }
        }
        let trap = crate::feature::NutTrap::default();
        let pocket = crate::build::nut_pocket(&trap, Some(3.0)).unwrap();
        assert!((pocket.across_flats - 5.8).abs() < 1e-6);
        assert!((pocket.depth - 2.7).abs() < 1e-6);
        let no_size = crate::build::nut_pocket(&trap, None).unwrap_err();
        assert!(no_size.contains("across-flats"), "{no_size}");
        let own = crate::feature::NutTrap {
            across_flats: Some(11.0),
            ..trap
        };
        assert!(
            crate::build::nut_pocket(&own, None).is_err(),
            "needs its depth"
        );
    }
}
