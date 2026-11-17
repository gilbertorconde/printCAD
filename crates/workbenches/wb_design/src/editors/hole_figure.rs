//! The drawing at the top of the Hole panel: the hole cut through its
//! material in section, the screw it is for seated in it (its head the
//! one the hole's cut makes room for, its nut in the nut trap), and the
//! hole's measures beside the parts they size. The measures of the field
//! last edited are drawn in the accent colour; a press on a measure
//! lights it the same way.
//!
//! Drawn in millimetres at the hole's true proportions, y up, the face
//! the hole opens on at y = 0 and its axis at x = 0.

use bench_api::{Callout, DiagramShape, DiagramStroke, Dimension, Widget};

use crate::feature::{DesignFeature, DrillPoint, HoleCut, NutSide, ThreadLength};
use crate::hole_tables::ScrewSeat;

/// The widget id of the drawing.
pub(crate) const FIGURE: &str = "hole figure";

/// The drawing of `feature`, a hole, its measures for `focus` (a field's
/// label) lit; `None` for any other feature.
pub(crate) fn hole_figure(feature: &DesignFeature, focus: Option<&str>) -> Option<Widget> {
    let hole = Hole::of(feature)?;
    let mut canvas = Canvas::new(focus);
    hole.draw(&mut canvas);
    Some(canvas.finish(FIGURE))
}

/// The field whose measure lies nearest `at`, a press on the drawing in
/// its own coordinates, if one lies near enough to mean it.
pub(crate) fn measure_at(feature: &DesignFeature, at: [f32; 2]) -> Option<String> {
    let hole = Hole::of(feature)?;
    let mut canvas = Canvas::new(None);
    hole.draw(&mut canvas);
    let reach = canvas.extent() * 0.08;
    let shift = canvas.shift();
    let near = |p: [f32; 2]| {
        let p = [p[0] + shift[0], p[1] + shift[1]];
        (p[0] - at[0]).hypot(p[1] - at[1])
    };
    canvas
        .marks
        .iter()
        .map(|m| (near(m.at), m))
        .filter(|(d, _)| *d <= reach)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, m)| m.key.to_string())
}

/// A field's label as the drawing keys it: without a colon, and without
/// the `#n` a second widget of the same name takes.
pub(crate) fn field_key(id: &str) -> &str {
    let id = id.split('#').next().unwrap_or(id);
    id.trim_end_matches(':')
}

/// What a measure stands for: the field it sizes, and where its words sit.
struct Mark {
    key: &'static str,
    at: [f32; 2],
}

/// The drawing as it is built up, in millimetres.
struct Canvas<'a> {
    focus: Option<&'a str>,
    shapes: Vec<DiagramShape>,
    dimensions: Vec<Dimension>,
    callouts: Vec<Callout>,
    marks: Vec<Mark>,
    min: [f32; 2],
    max: [f32; 2],
}

fn pt(p: [f64; 2]) -> [f32; 2] {
    [p[0] as f32, p[1] as f32]
}

impl<'a> Canvas<'a> {
    fn new(focus: Option<&'a str>) -> Self {
        Self {
            focus: focus.map(field_key),
            shapes: Vec::new(),
            dimensions: Vec::new(),
            callouts: Vec::new(),
            marks: Vec::new(),
            min: [f32::MAX; 2],
            max: [f32::MIN; 2],
        }
    }

    fn cover(&mut self, p: [f32; 2]) {
        self.min = [self.min[0].min(p[0]), self.min[1].min(p[1])];
        self.max = [self.max[0].max(p[0]), self.max[1].max(p[1])];
    }

    fn lit(&self, keys: &[&str]) -> bool {
        self.focus.is_some_and(|f| keys.contains(&f))
    }

    fn path(&mut self, points: &[[f64; 2]], closed: bool, stroke: DiagramStroke, fill: bool) {
        let points: Vec<[f32; 2]> = points.iter().map(|p| pt(*p)).collect();
        for p in &points {
            self.cover(*p);
        }
        self.shapes.push(DiagramShape::Path {
            points,
            closed,
            stroke,
            fill,
        });
    }

    fn line(&mut self, points: &[[f64; 2]], stroke: DiagramStroke) {
        self.path(points, false, stroke, false);
    }

    /// A shaded outline; keep it convex.
    fn solid(&mut self, points: &[[f64; 2]]) {
        self.path(points, true, DiagramStroke::Outline, true);
    }

    /// A measure from `from` to `to`, its line `offset` to the left of
    /// that way (negative: the right), lit while one of `keys` is edited.
    fn dim(
        &mut self,
        keys: &[&'static str],
        from: [f64; 2],
        to: [f64; 2],
        offset: f64,
        text: String,
    ) {
        let (f, t) = (pt(from), pt(to));
        let (dx, dy) = (t[0] - f[0], t[1] - f[1]);
        let len = dx.hypot(dy).max(1e-6);
        let n = [-dy / len * offset as f32, dx / len * offset as f32];
        let edge = 1.3;
        self.cover([f[0] + n[0] * edge, f[1] + n[1] * edge]);
        self.cover([t[0] + n[0] * edge, t[1] + n[1] * edge]);
        let mid = [(f[0] + t[0]) / 2.0 + n[0], (f[1] + t[1]) / 2.0 + n[1]];
        self.marks.push(Mark {
            key: keys[0],
            at: mid,
        });
        let emphasis = self.lit(keys);
        self.dimensions.push(Dimension {
            from: f,
            to: t,
            offset: offset as f32,
            text,
            emphasis,
        });
    }

    /// A note pointing at `anchor`, its words at `at`.
    fn callout(&mut self, keys: &[&'static str], anchor: [f64; 2], at: [f64; 2], text: String) {
        let (anchor, at) = (pt(anchor), pt(at));
        self.cover(anchor);
        self.cover(at);
        self.marks.push(Mark { key: keys[0], at });
        let emphasis = self.lit(keys);
        self.callouts.push(Callout {
            anchor,
            at,
            text,
            emphasis,
        });
    }

    fn extent(&self) -> f32 {
        (self.max[0] - self.min[0])
            .max(self.max[1] - self.min[1])
            .max(1.0)
    }

    fn margin(&self) -> f32 {
        (self.extent() * 0.12).max(0.5)
    }

    /// How far everything moves to sit in the margin.
    fn shift(&self) -> [f32; 2] {
        let margin = self.margin();
        [margin - self.min[0], margin - self.min[1]]
    }

    fn finish(mut self, id: &str) -> Widget {
        if self.min[0] > self.max[0] {
            self.min = [0.0; 2];
            self.max = [1.0; 2];
        }
        // Room for each note's words round where they sit, at about the
        // size they take in a drawing the panel's width.
        let (tall, char_wide) = (self.extent() * 0.05, self.extent() * 0.025);
        let words: Vec<([f32; 2], usize)> = self
            .callouts
            .iter()
            .map(|c| (c.at, c.text.chars().count()))
            .collect();
        for (at, chars) in words {
            let half = char_wide * chars as f32 / 2.0 + char_wide;
            self.cover([at[0] - half, at[1] - tall]);
            self.cover([at[0] + half, at[1] + tall]);
        }
        let margin = self.margin();
        let shift = self.shift();
        let mv = |p: &mut [f32; 2]| {
            p[0] += shift[0];
            p[1] += shift[1];
        };
        for shape in &mut self.shapes {
            match shape {
                DiagramShape::Path { points, .. } => points.iter_mut().for_each(mv),
                DiagramShape::Circle { center, .. } => mv(center),
                DiagramShape::Text { at, .. } => mv(at),
            }
        }
        for d in &mut self.dimensions {
            mv(&mut d.from);
            mv(&mut d.to);
        }
        for c in &mut self.callouts {
            mv(&mut c.anchor);
            mv(&mut c.at);
        }
        Widget::Diagram {
            id: id.into(),
            width: self.max[0] - self.min[0] + 2.0 * margin,
            height: self.max[1] - self.min[1] + 2.0 * margin,
            shapes: self.shapes,
            dimensions: self.dimensions,
            callouts: self.callouts,
        }
    }
}

/// A length as a measure reads it: at most two decimals, no trailing
/// zeros.
fn mm(v: f64) -> String {
    let text = format!("{v:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_string()
}

fn deg(v: f64) -> String {
    format!("{}°", mm(v))
}

/// A stretch of the hole's wall: its radius going linearly from `r_top`
/// at `y_top` to `r_bot` at `y_bot`, below it.
#[derive(Clone, Copy)]
struct Piece {
    y_top: f64,
    y_bot: f64,
    r_top: f64,
    r_bot: f64,
}

impl Piece {
    fn r(&self, y: f64) -> f64 {
        let span = self.y_top - self.y_bot;
        if span <= 1e-12 {
            return self.r_top.max(self.r_bot);
        }
        self.r_bot + (self.r_top - self.r_bot) * (y - self.y_bot) / span
    }

    fn covers(&self, top: f64, bot: f64) -> bool {
        self.y_top >= top - 1e-9 && self.y_bot <= bot + 1e-9
    }
}

/// A band of the hole's depth: its top and bottom height, and the wall's
/// radius at each, `None` past the hole's end.
struct Band {
    top: f64,
    bot: f64,
    radii: Option<(f64, f64)>,
}

/// The cut round the hole's mouth as it is drawn, and the fields that
/// size it.
#[derive(Clone, Copy)]
enum Mouth {
    None,
    /// A counterbore or a spotface: radius, depth.
    Bore {
        r: f64,
        depth: f64,
        keys: [&'static str; 2],
    },
    /// A countersink: radius at the face, included angle.
    Sink {
        r: f64,
        angle: f64,
        keys: [&'static str; 2],
    },
    /// A counterdrill: radius, depth of its straight part, included angle.
    Drill {
        r: f64,
        depth: f64,
        angle: f64,
    },
}

/// The screw head the drawing seats.
#[derive(Clone, Copy, PartialEq)]
enum Head {
    Cap,
    LowCap,
    Button,
    Hex,
    Countersunk,
}

/// The screw's thread as the drawing needs it.
struct Screw {
    major: f64,
    minor: f64,
    designation: String,
    head: Head,
    washer: bool,
}

/// The nut trap as drawn.
struct Nut {
    /// Half the pocket's across-flats, and its depth.
    half: f64,
    depth: f64,
    top: bool,
    /// The nut itself, across its flats and thick, when its standard
    /// names one.
    nut: Option<(f64, f64)>,
}

/// Everything the drawing reads from a hole, its numbers resolved.
struct Hole {
    /// The drill's radius at the face.
    r: f64,
    /// Narrowing inward, degrees; with the words its callout reads.
    taper: f64,
    taper_text: Option<String>,
    /// How deep, or `None` through.
    depth: Option<f64>,
    /// The material's thickness drawn.
    plate: f64,
    /// The drill point's angle and whether the depth runs to its tip.
    point: Option<(f64, bool)>,
    mouth: Mouth,
    nut: Option<Nut>,
    screw: Option<Screw>,
    /// Tapped, and how far down from the face the thread runs.
    tapped: Option<f64>,
    /// The tapped thread's length is its own field.
    thread_given: bool,
    /// The keys of the drill's diameter.
    drill_keys: &'static [&'static str],
}

const STANDARD_KEYS: &[&str] = &["Size", "Standard", "Class", "Threaded (tap drill)", "Fit"];
const CUSTOM_KEYS: &[&str] = &["Diameter"];
const CLEARANCE_KEYS: &[&str] = &["Clearance", "Fit", "Size", "Standard"];

impl Hole {
    fn of(feature: &DesignFeature) -> Option<Self> {
        let DesignFeature::Hole {
            depth,
            through_all,
            cut,
            thread,
            threaded,
            modeled_thread,
            thread_depth,
            clearance,
            thread_length,
            drill_point,
            point_in_depth,
            nut_trap,
            ..
        } = feature
        else {
            return None;
        };
        let drilled = f64::from(crate::build::hole_diameter(feature));
        if drilled <= 0.0 {
            return None;
        }
        let r = drilled / 2.0;
        let size = thread.as_ref().and_then(|t| t.resolve().ok());
        let depth_mm = f64::from(*depth).max(r * 0.5);
        let taper = crate::build::hole_taper_deg(feature);
        let standard_taper = *threaded && thread.as_ref().is_some_and(|t| t.standard.is_tapered());
        let taper_text = if standard_taper {
            Some("1:16 taper".to_string())
        } else if taper.abs() > 1e-9 {
            Some(format!("taper {}", deg(taper)))
        } else {
            None
        };
        let metric_major = thread
            .as_ref()
            .filter(|t| t.standard.is_metric())
            .and(size.as_ref())
            .map(|s| s.major);
        let (mouth, seat) = match *cut {
            HoleCut::None => (Mouth::None, None),
            HoleCut::Counterbore { diameter, depth } => (
                Mouth::Bore {
                    r: f64::from(diameter) / 2.0,
                    depth: f64::from(depth),
                    keys: ["Bore Ø", "Bore depth"],
                },
                None,
            ),
            HoleCut::Spotface { diameter, depth } => (
                Mouth::Bore {
                    r: f64::from(diameter) / 2.0,
                    depth: f64::from(depth),
                    keys: ["Spot Ø", "Spot depth"],
                },
                None,
            ),
            HoleCut::Countersink {
                diameter,
                angle_deg,
            } => (
                Mouth::Sink {
                    r: f64::from(diameter) / 2.0,
                    angle: f64::from(angle_deg),
                    keys: ["Sink Ø", "Sink angle"],
                },
                None,
            ),
            HoleCut::Counterdrill {
                diameter,
                depth,
                angle_deg,
            } => (
                Mouth::Drill {
                    r: f64::from(diameter) / 2.0,
                    depth: f64::from(depth),
                    angle: f64::from(angle_deg),
                },
                None,
            ),
            HoleCut::Seat { seat } => {
                let made = metric_major.and_then(|d| seat.cut(d));
                let mouth = match made {
                    Some(HoleCut::Counterbore { diameter, depth }) => Mouth::Bore {
                        r: f64::from(diameter) / 2.0,
                        depth: f64::from(depth),
                        keys: ["Hole cut", "Hole cut"],
                    },
                    Some(HoleCut::Countersink {
                        diameter,
                        angle_deg,
                    }) => Mouth::Sink {
                        r: f64::from(diameter) / 2.0,
                        angle: f64::from(angle_deg),
                        keys: ["Hole cut", "Hole cut"],
                    },
                    _ => Mouth::None,
                };
                (mouth, Some(seat))
            }
        };
        let nominal = metric_major;
        let nut = nut_trap.as_ref().and_then(|trap| {
            let pocket = crate::build::nut_pocket(trap, nominal).ok()?;
            let nut = nominal
                .and_then(|d| trap.standard.nut(d))
                .map(|n| (n.across_flats, n.thickness));
            Some(Nut {
                half: pocket.across_flats / 2.0,
                depth: pocket.depth,
                top: trap.side == NutSide::Top,
                nut,
            })
        });
        let point = match drill_point {
            DrillPoint::Angled { angle_deg } if !*through_all => {
                Some((f64::from(*angle_deg), *point_in_depth))
            }
            _ => None,
        };
        let head = match (seat, mouth) {
            (Some(ScrewSeat::ButtonHead), _) => Head::Button,
            (Some(ScrewSeat::LowHeadCap), _) => Head::LowCap,
            (Some(ScrewSeat::HexHead), _) => Head::Hex,
            (_, Mouth::Sink { .. }) => Head::Countersunk,
            _ => Head::Cap,
        };
        let screw = thread
            .as_ref()
            .zip(size.as_ref())
            .map(|(spec, size)| Screw {
                major: size.major,
                minor: size.minor,
                designation: spec.designation(),
                head,
                washer: seat == Some(ScrewSeat::CapScrewWithWasher),
            });
        let plate = if *through_all {
            depth_mm.max(drilled * 1.5)
        } else {
            depth_mm
        };
        let thread_given = *threaded && *modeled_thread && *thread_length == ThreadLength::Given;
        let tapped = (*threaded && screw.is_some()).then(|| {
            let whole = plate;
            let pitch = size.as_ref().map_or(0.0, |s| s.pitch);
            let run = match (*modeled_thread, thread_length) {
                (true, ThreadLength::Given) => f64::from(*thread_depth),
                (true, ThreadLength::RunOut) => whole - 3.0 * pitch,
                _ => whole,
            };
            run.clamp(pitch.max(0.1), whole)
        });
        let drill_keys = match (thread.is_some(), clearance.is_some()) {
            (false, _) => CUSTOM_KEYS,
            (true, true) => CLEARANCE_KEYS,
            (true, false) => STANDARD_KEYS,
        };
        Some(Self {
            r,
            taper,
            taper_text,
            depth: (!*through_all).then_some(depth_mm),
            plate: plate
                + if *through_all {
                    0.0
                } else {
                    // Material left under a blind hole's bottom.
                    (drilled * 0.6).max(depth_mm * 0.15)
                },
            point,
            mouth,
            nut,
            screw,
            tapped,
            thread_given,
            drill_keys,
        })
    }

    /// The wall's stretches, top down.
    fn pieces(&self) -> Vec<Piece> {
        let tan_taper = self.taper.to_radians().tan();
        let mut pieces = Vec::new();
        let r_at = |down: f64| (self.r - down * tan_taper).max(0.0);
        match self.depth {
            None => pieces.push(Piece {
                y_top: 0.0,
                y_bot: -self.plate,
                r_top: self.r,
                r_bot: r_at(self.plate),
            }),
            Some(depth) => match self.point {
                None => pieces.push(Piece {
                    y_top: 0.0,
                    y_bot: -depth,
                    r_top: self.r,
                    r_bot: r_at(depth),
                }),
                Some((angle, within)) => {
                    let tan_half = (angle * 0.5).to_radians().tan().max(1e-6);
                    let wall = if within {
                        ((depth - self.r / tan_half) / (1.0 - tan_taper / tan_half)).max(0.0)
                    } else {
                        depth
                    };
                    let rb = r_at(wall);
                    pieces.push(Piece {
                        y_top: 0.0,
                        y_bot: -wall,
                        r_top: self.r,
                        r_bot: rb,
                    });
                    pieces.push(Piece {
                        y_top: -wall,
                        y_bot: -wall - rb / tan_half,
                        r_top: rb,
                        r_bot: 0.0,
                    });
                }
            },
        }
        match self.mouth {
            Mouth::None => {}
            Mouth::Bore { r, depth, .. } => pieces.push(Piece {
                y_top: 0.0,
                y_bot: -depth,
                r_top: r,
                r_bot: r,
            }),
            Mouth::Sink { r, angle, .. } => {
                let tan_half = (angle * 0.5).to_radians().tan().max(1e-6);
                pieces.push(Piece {
                    y_top: 0.0,
                    y_bot: -(r - self.r).max(0.0) / tan_half,
                    r_top: r,
                    r_bot: self.r.min(r),
                });
            }
            Mouth::Drill { r, depth, angle } => {
                let tan_half = (angle * 0.5).to_radians().tan().max(1e-6);
                pieces.push(Piece {
                    y_top: 0.0,
                    y_bot: -depth,
                    r_top: r,
                    r_bot: r,
                });
                pieces.push(Piece {
                    y_top: -depth,
                    y_bot: -depth - (r - self.r).max(0.0) / tan_half,
                    r_top: r,
                    r_bot: self.r.min(r),
                });
            }
        }
        if let Some(nut) = &self.nut {
            let (top, bot) = if nut.top {
                (0.0, -nut.depth)
            } else {
                let end = self.depth.unwrap_or(self.plate);
                (-end + nut.depth, -end)
            };
            pieces.push(Piece {
                y_top: top,
                y_bot: bot,
                r_top: nut.half,
                r_bot: nut.half,
            });
        }
        pieces.retain(|p| p.y_top > p.y_bot + 1e-9);
        pieces
    }

    /// The wall as bands top down, each with the widest stretch there.
    fn bands(&self) -> Vec<Band> {
        let pieces = self.pieces();
        let mut levels: Vec<f64> = vec![0.0, -self.plate];
        for p in &pieces {
            levels.push(p.y_top);
            levels.push(p.y_bot);
        }
        // Where two stretches cross, the wider one changes.
        for (i, a) in pieces.iter().enumerate() {
            for b in &pieces[i + 1..] {
                let (top, bot) = (a.y_top.min(b.y_top), a.y_bot.max(b.y_bot));
                if top <= bot {
                    continue;
                }
                let (da, db) = (a.r(top) - b.r(top), a.r(bot) - b.r(bot));
                if da * db < 0.0 {
                    levels.push(top - (top - bot) * da / (da - db));
                }
            }
        }
        levels.retain(|y| *y <= 1e-9 && *y >= -self.plate - 1e-9);
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        levels
            .windows(2)
            .map(|w| {
                let (top, bot) = (w[0], w[1]);
                let widest = |y: f64| {
                    pieces
                        .iter()
                        .filter(|p| p.covers(top, bot))
                        .map(|p| p.r(y))
                        .fold(None, |m: Option<f64>, r| Some(m.map_or(r, |m| m.max(r))))
                };
                Band {
                    top,
                    bot,
                    radii: widest(top).zip(widest(bot)),
                }
            })
            .collect()
    }

    /// The wall from the face down, as points on its right side; and
    /// whether it opens out of the material's far side.
    fn wall(&self) -> (Vec<[f64; 2]>, bool) {
        let mut points: Vec<[f64; 2]> = Vec::new();
        let mut through = false;
        let push = |points: &mut Vec<[f64; 2]>, p: [f64; 2]| {
            if points
                .last()
                .is_none_or(|q| (q[0] - p[0]).abs() > 1e-9 || (q[1] - p[1]).abs() > 1e-9)
            {
                points.push(p);
            }
        };
        for Band { top, bot, radii } in self.bands() {
            match radii {
                Some((rt, rb)) => {
                    push(&mut points, [rt, top]);
                    push(&mut points, [rb, bot]);
                    through = bot <= -self.plate + 1e-9;
                }
                None => {
                    if let Some(last) = points.last().copied()
                        && last[0] > 1e-9
                    {
                        push(&mut points, [0.0, last[1]]);
                    }
                    break;
                }
            }
        }
        (points, through)
    }

    /// The half-width of the material drawn.
    fn half_width(&self) -> f64 {
        let widest = self
            .pieces()
            .iter()
            .map(|p| p.r_top.max(p.r_bot))
            .fold(self.r, f64::max);
        let screw = self.screw.as_ref().map_or(0.0, |s| s.major);
        (widest * 1.6).max(widest + screw.max(self.r * 2.0) * 0.8)
    }

    fn draw(&self, c: &mut Canvas) {
        let w = self.half_width();
        let bottom = -self.plate;
        self.hatch(c, w);
        let (wall, through) = self.wall();
        let r0 = wall.first().map_or(self.r, |p| p[0]);
        for side in [1.0, -1.0] {
            let mut path = vec![[side * w, 0.0], [side * r0, 0.0]];
            path.extend(wall.iter().map(|p| [side * p[0], p[1]]));
            if through {
                path.push([side * w, bottom]);
            }
            c.line(&path, DiagramStroke::Outline);
            c.line(&[[side * w, 0.0], [side * w, bottom]], DiagramStroke::Thin);
        }
        if !through {
            c.line(&[[-w, bottom], [w, bottom]], DiagramStroke::Outline);
        }
        if let Some(run) = self.tapped
            && let Some(screw) = &self.screw
        {
            let rm = screw.major / 2.0;
            let top = self.mouth_floor();
            for side in [1.0, -1.0] {
                c.line(&[[side * rm, top], [side * rm, -run]], DiagramStroke::Thin);
            }
        }
        let head_top = self.draw_screw(c);
        if let Some(nut) = &self.nut {
            self.draw_nut(c, nut);
        }
        let low = c.min[1] as f64;
        c.line(
            &[
                [0.0, head_top.max(0.0) + self.r * 0.6],
                [0.0, low - self.r * 0.6],
            ],
            DiagramStroke::Axis,
        );
        self.measures(c, w, head_top.max(0.0), through);
    }

    /// 45° lines over the material, band by band.
    fn hatch(&self, c: &mut Canvas, w: f64) {
        let mut cells: Vec<Vec<[f64; 2]>> = Vec::new();
        let mut end = 0.0;
        for Band { top, bot, radii } in self.bands() {
            match radii {
                Some((rt, rb)) => {
                    for side in [1.0, -1.0] {
                        cells.push(vec![
                            [side * rt, top],
                            [side * w, top],
                            [side * w, bot],
                            [side * rb, bot],
                        ]);
                    }
                }
                None => cells.push(vec![[-w, top], [w, top], [w, bot], [-w, bot]]),
            }
            end = bot;
        }
        if end > -self.plate + 1e-9 {
            cells.push(vec![
                [-w, end],
                [w, end],
                [w, -self.plate],
                [-w, -self.plate],
            ]);
        }
        let step = (2.0 * w / 14.0).max(0.05) * std::f64::consts::SQRT_2;
        let (lo, hi) = (-w - 0.0, w + self.plate);
        let mut k = (lo / step).floor();
        while k * step <= hi {
            let at = k * step;
            for cell in &cells {
                if let Some(segment) = clip(cell, at) {
                    c.line(&segment, DiagramStroke::Thin);
                }
            }
            k += 1.0;
        }
    }

    /// Where a screw head bears: the floor of the mouth's cut, else the
    /// face.
    fn mouth_floor(&self) -> f64 {
        match self.mouth {
            Mouth::Bore { depth, .. } | Mouth::Drill { depth, .. } => -depth,
            Mouth::None | Mouth::Sink { .. } => 0.0,
        }
    }

    /// The screw goes in from the far side, into a nut trapped at the
    /// mouth of a through hole.
    fn seated_from_far_side(&self) -> bool {
        self.nut.as_ref().is_some_and(|n| n.top) && self.depth.is_none()
    }

    /// Where the shank of a screw `d` across ends: below the face when
    /// seated from it, near the face when from the far side.
    fn screw_tip(&self, d: f64) -> f64 {
        if let Some(run) = self.tapped {
            -(run * 0.85).max(d * 0.5)
        } else if self.seated_from_far_side() {
            -self.nut.as_ref().map_or(0.0, |n| n.depth) * 0.15
        } else {
            match self.depth {
                Some(depth) => -(depth - self.r * 0.3).max(d * 0.5),
                None => match &self.nut {
                    Some(nut) if !nut.top => -self.plate,
                    _ => -self.plate - d * 1.2,
                },
            }
        }
    }

    /// The screw, seated: from the face in, or, with the nut trapped at
    /// the mouth, from the far side. Answers the height of its top.
    fn draw_screw(&self, c: &mut Canvas) -> f64 {
        let Some(screw) = &self.screw else {
            return 0.0;
        };
        let d = screw.major;
        let from_below = self.seated_from_far_side();
        let tip = self.screw_tip(d);
        if from_below {
            let bear = -self.plate;
            self.head(c, screw, bear, -1.0);
            self.shank(c, screw, bear, tip);
            return 0.0;
        }
        let bear = self.mouth_floor();
        let bear = if screw.washer {
            let t = (d * 0.18).max(0.3);
            let r = (d * 1.0)
                .min(self.mouth_radius().unwrap_or(d) - 0.2)
                .max(d * 0.6);
            c.solid(&[[-r, bear], [r, bear], [r, bear + t], [-r, bear + t]]);
            bear + t
        } else {
            bear
        };
        let (shank_top, top) = self.head(c, screw, bear, 1.0);
        self.shank(c, screw, shank_top, tip);
        top
    }

    /// The mouth's cut radius, when it has one.
    fn mouth_radius(&self) -> Option<f64> {
        match self.mouth {
            Mouth::None => None,
            Mouth::Bore { r, .. } | Mouth::Sink { r, .. } | Mouth::Drill { r, .. } => Some(r),
        }
    }

    /// The head bearing at `bear`, going up (`dir` 1) or down (-1) from
    /// it; answers where the shank starts and the head's far end.
    fn head(&self, c: &mut Canvas, screw: &Screw, bear: f64, dir: f64) -> (f64, f64) {
        let d = screw.major;
        let room = self.mouth_radius().filter(|_| dir > 0.0);
        let fit = |want: f64| room.map_or(want, |r| want.min(r * 2.0 - d * 0.1).max(d * 1.1));
        match screw.head {
            Head::Countersunk => {
                let Mouth::Sink { r, angle, .. } = self.mouth else {
                    return (bear, bear);
                };
                let rk = (r - d * 0.05).max(d / 2.0);
                let tan_half = (angle * 0.5).to_radians().tan().max(1e-6);
                let h = (rk - d / 2.0) / tan_half;
                c.solid(&[[-rk, 0.0], [rk, 0.0], [d / 2.0, -h], [-d / 2.0, -h]]);
                self.socket(c, 0.0, d * 0.55, h * 0.7);
                (-h, 0.0)
            }
            head => {
                let (dk, k) = match head {
                    Head::Cap => (fit(d * 1.5), d),
                    Head::LowCap => (fit(d * 1.5), d * 0.65),
                    Head::Button => (fit(d * 1.75), d * 0.55),
                    _ => (fit(d * 1.7), d * 0.7),
                };
                let (a, b) = (bear, bear + dir * k);
                let rk = dk / 2.0;
                match head {
                    Head::Button => {
                        let mut dome = Vec::new();
                        for i in 0..=12 {
                            let t = std::f64::consts::PI * f64::from(i) / 12.0;
                            dome.push([rk * t.cos(), a + dir * k * t.sin()]);
                        }
                        c.solid(&dome);
                    }
                    _ => c.solid(&[[-rk, a], [rk, a], [rk, b], [-rk, b]]),
                }
                if head == Head::Hex {
                    for side in [1.0, -1.0] {
                        c.line(
                            &[[side * rk * 0.5, a], [side * rk * 0.5, b]],
                            DiagramStroke::Thin,
                        );
                    }
                } else {
                    let across = if head == Head::Button { 0.6 } else { 0.8 };
                    self.socket(c, b, d * across, k * 0.5 * dir);
                }
                (a, b)
            }
        }
    }

    /// A hex key's socket, `across` wide and `deep` into the head from its
    /// end at `end`, behind the surface.
    fn socket(&self, c: &mut Canvas, end: f64, across: f64, deep: f64) {
        let h = across / 2.0;
        c.line(
            &[
                [-h, end],
                [-h, end - deep],
                [0.0, end - deep - h * 0.5_f64.copysign(deep)],
                [h, end - deep],
                [h, end],
            ],
            DiagramStroke::Hidden,
        );
    }

    /// The shank from `from` to `to`, its thread's root lines along its
    /// end.
    fn shank(&self, c: &mut Canvas, screw: &Screw, from: f64, to: f64) {
        let rd = screw.major / 2.0;
        let chamfer = (rd * 0.15).min((from - to).abs() * 0.2);
        let s = (to - from).signum();
        c.solid(&[
            [-rd, from],
            [rd, from],
            [rd, to - s * chamfer],
            [rd - chamfer, to],
            [-rd + chamfer, to],
            [-rd, to - s * chamfer],
        ]);
        let rm = (screw.minor / 2.0).min(rd);
        let length = (to - from).abs();
        let threaded = match self.tapped {
            Some(_) => length,
            None => length.min(screw.major * 2.0 + 6.0),
        };
        let start = to - s * threaded;
        for side in [1.0, -1.0] {
            c.line(
                &[[side * rm, start], [side * rm, to - s * chamfer]],
                DiagramStroke::Thin,
            );
        }
    }

    fn draw_nut(&self, c: &mut Canvas, nut: &Nut) {
        let Some((af, thick)) = nut.nut else {
            return;
        };
        let half = (af / 2.0).min(nut.half);
        let thick = thick.min(nut.depth);
        let (a, b) = if nut.top {
            (-nut.depth, -nut.depth + thick)
        } else {
            let end = -self.depth.unwrap_or(self.plate);
            (end + nut.depth - thick, end + nut.depth)
        };
        let ch = (thick * 0.15).min(half * 0.15);
        c.solid(&[
            [-half + ch, a],
            [half - ch, a],
            [half, a + ch],
            [half, b - ch],
            [half - ch, b],
            [-half + ch, b],
            [-half, b - ch],
            [-half, a + ch],
        ]);
        for side in [1.0, -1.0] {
            c.line(
                &[[side * half * 0.5, a], [side * half * 0.5, b]],
                DiagramStroke::Thin,
            );
        }
    }

    /// The hole's measures, each beside what it sizes; `top` is the
    /// highest the drawing reaches above the face.
    fn measures(&self, c: &mut Canvas, w: f64, top: f64, through: bool) {
        let so = (c.extent() as f64 * 0.11).max(0.8);
        let bottom = -self.plate;
        let mut above = top + so;
        let mut left = 1.0;
        // The drill.
        let drill = format!("Ø{}", mm(self.r * 2.0));
        if through {
            let rb = self.wall().0.last().map_or(self.r, |p| p[0]);
            c.dim(self.drill_keys, [-rb, bottom], [rb, bottom], -so, drill);
        } else {
            let wall = self.wall().0;
            let y = wall
                .iter()
                .rev()
                .find(|p| p[0] >= self.r * 0.5)
                .map_or(-self.r, |p| p[1])
                .max(-self.depth.unwrap_or(0.0))
                + self.r * 0.6;
            let ry = self.pieces().first().map_or(self.r, |p| p.r(y));
            c.dim(self.drill_keys, [-ry, y], [ry, y], 0.0, drill);
        }
        // How deep.
        match self.depth {
            Some(depth) => {
                let to = match self.point {
                    Some((_, false)) => {
                        let wall = self.pieces();
                        wall.first().map_or(-depth, |p| p.y_bot)
                    }
                    _ => -depth,
                };
                c.dim(
                    &["Depth", "Point within the depth"],
                    [w, 0.0],
                    [w, to],
                    so,
                    mm(depth),
                );
            }
            None => c.callout(
                &["Through all"],
                [w * 0.6, bottom],
                [w + so * 1.2, bottom - so * 1.3],
                "through".to_string(),
            ),
        }
        // The cut round the mouth.
        match self.mouth {
            Mouth::None => {}
            Mouth::Bore { r, depth, keys } => {
                c.dim(
                    &[keys[0], "Hole cut"],
                    [-r, 0.0],
                    [r, 0.0],
                    above,
                    format!("Ø{}", mm(r * 2.0)),
                );
                above += so * 1.2;
                c.dim(
                    &[keys[1], "Hole cut"],
                    [-w, 0.0],
                    [-w, -depth],
                    -so * left,
                    mm(depth),
                );
                left += 1.4;
            }
            Mouth::Sink { r, angle, keys } => {
                c.dim(
                    &[keys[0], "Hole cut"],
                    [-r, 0.0],
                    [r, 0.0],
                    above,
                    format!("Ø{}", mm(r * 2.0)),
                );
                above += so * 1.2;
                let tan_half = (angle * 0.5).to_radians().tan().max(1e-6);
                let h = (r - self.r) / tan_half;
                c.callout(
                    &[keys[1], "Hole cut"],
                    [-(r + self.r) / 2.0, -h / 2.0],
                    [-w - so * left, -h / 2.0 + so * 0.4],
                    deg(angle),
                );
                left += 1.4;
            }
            Mouth::Drill { r, depth, angle } => {
                c.dim(
                    &["Counterdrill Ø", "Hole cut"],
                    [-r, 0.0],
                    [r, 0.0],
                    above,
                    format!("Ø{}", mm(r * 2.0)),
                );
                above += so * 1.2;
                c.dim(
                    &["Counterdrill depth", "Hole cut"],
                    [-w, 0.0],
                    [-w, -depth],
                    -so * left,
                    mm(depth),
                );
                left += 1.4;
                let tan_half = (angle * 0.5).to_radians().tan().max(1e-6);
                let h = (r - self.r) / tan_half;
                c.callout(
                    &["Counterdrill angle", "Hole cut"],
                    [-(r + self.r) / 2.0, -depth - h / 2.0],
                    [-w - so * left, -depth - h / 2.0 - so * 0.4],
                    deg(angle),
                );
            }
        }
        // The drill's point.
        if let Some((angle, _)) = self.point {
            let tip = self.pieces().get(1).map_or(-self.plate, |p| p.y_bot);
            c.callout(
                &["Point angle", "Drill point"],
                [0.0, tip],
                [self.r + so, tip - so * 0.6],
                deg(angle),
            );
        }
        // The taper.
        if let Some(text) = &self.taper_text {
            let y = -self.depth.unwrap_or(self.plate) * 0.6;
            let r = self.pieces().first().map_or(self.r, |p| p.r(y));
            c.callout(&["Taper"], [-r, y], [-w - so * left, y], text.clone());
        }
        // The nut trap.
        if let Some(nut) = &self.nut {
            let keys: &[&'static str] = &["Across flats", "Nut clearance", "Nut"];
            let text = format!("AF {}", mm(nut.half * 2.0));
            if nut.top {
                c.dim(keys, [-nut.half, 0.0], [nut.half, 0.0], above, text);
                c.dim(
                    &["Nut depth", "Own depth"],
                    [-w, 0.0],
                    [-w, -nut.depth],
                    -so * left,
                    mm(nut.depth),
                );
            } else {
                let end = -self.depth.unwrap_or(self.plate);
                let low = if through { -so * 2.4 } else { -so };
                c.dim(keys, [-nut.half, bottom], [nut.half, bottom], low, text);
                c.dim(
                    &["Nut depth", "Own depth"],
                    [-w, end + nut.depth],
                    [-w, end],
                    -so * left,
                    mm(nut.depth),
                );
            }
            left += 1.4;
        }
        // The thread.
        if let Some(screw) = &self.screw {
            if let Some(run) = self.tapped
                && self.thread_given
            {
                c.dim(
                    &["Thread depth", "Thread length", "Modeled thread"],
                    [-w, 0.0],
                    [-w, -run],
                    -so * left,
                    mm(run),
                );
            }
            let y = match self.tapped {
                Some(run) => -run * 0.2,
                // Along the thread, near the shank's end, and within the
                // part, clear of the note under it.
                None => {
                    let tip = self.screw_tip(screw.major);
                    let start = if self.seated_from_far_side() {
                        -self.plate
                    } else {
                        self.mouth_floor()
                    };
                    (tip + (start - tip) * 0.3).max(bottom + so)
                }
            };
            // The words right of the depth's measure, their leader kept off
            // its words, which sit halfway down it.
            let (x0, x1) = (screw.major / 2.0, w + so * 2.6);
            let mut at_y = y + so * 0.5;
            if let Some(depth) = self.depth {
                let words = -depth / 2.0;
                let along = (w + so - x0) / (x1 - x0);
                let leader = |at_y: f64| y + (at_y - y) * along;
                while (leader(at_y) - words).abs() < so * 0.9 || (at_y - words).abs() < so * 0.9 {
                    at_y += so * 0.5;
                }
            }
            c.callout(
                &["Size", "Standard", "Class", "Left-hand thread"],
                [x0, y],
                [x1, at_y],
                screw.designation.clone(),
            );
        }
    }
}

/// The part of the 45° line x − y = `at` inside the convex `cell`.
fn clip(cell: &[[f64; 2]], at: f64) -> Option<[[f64; 2]; 2]> {
    let side = |p: [f64; 2]| p[0] - p[1] - at;
    let mut hits = Vec::new();
    for i in 0..cell.len() {
        let (a, b) = (cell[i], cell[(i + 1) % cell.len()]);
        let (fa, fb) = (side(a), side(b));
        if (fa <= 0.0) != (fb <= 0.0) {
            let t = fa / (fa - fb);
            hits.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    match hits.as_slice() {
        [p, q] if (p[0] - q[0]).hypot(p[1] - q[1]) > 1e-9 => Some([*p, *q]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature::ThreadSpec;
    use crate::hole_tables::ThreadStandard;

    fn hole(edit: impl FnOnce(&mut DesignFeature)) -> DesignFeature {
        let mut feature: DesignFeature = serde_json::from_value(serde_json::json!({
            "Hole": {
                "sketch": uuid::Uuid::nil(), "diameter": 6.0, "depth": 10.0,
                "through_all": false, "cut": "None", "drill_point": "Flat"
            }
        }))
        .unwrap();
        edit(&mut feature);
        feature
    }

    fn parts(widget: &Widget) -> (&[Dimension], &[Callout]) {
        let Widget::Diagram {
            dimensions,
            callouts,
            ..
        } = widget
        else {
            panic!("not a drawing")
        };
        (dimensions, callouts)
    }

    /// An M5 hole with the ISO 4762 seat reads the seat's 10 mm bore and
    /// 5.4 mm depth beside it, its drill, and the thread it is for; the
    /// field last edited lights its own measure only.
    #[test]
    fn a_seated_hole_reads_its_seat_and_lights_the_field_edited() {
        let feature = hole(|f| {
            if let DesignFeature::Hole { thread, cut, .. } = f {
                *thread = Some(ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M5"));
                *cut = HoleCut::Seat {
                    seat: ScrewSeat::SocketHead,
                };
            }
        });
        let drawing = hole_figure(&feature, Some("Depth")).unwrap();
        let (dimensions, callouts) = parts(&drawing);
        let texts: Vec<&str> = dimensions.iter().map(|d| d.text.as_str()).collect();
        for want in ["Ø10", "5.4", "Ø5.5", "10"] {
            assert!(texts.contains(&want), "{want} in {texts:?}");
        }
        assert!(callouts.iter().any(|c| c.text == "M5-6H"));
        let lit: Vec<&str> = dimensions
            .iter()
            .filter(|d| d.emphasis)
            .map(|d| d.text.as_str())
            .collect();
        assert_eq!(lit, ["10"], "only the depth lit");
    }

    /// A press on a measure names the field it sizes; one far from every
    /// measure names none.
    #[test]
    fn a_press_on_a_measure_names_its_field() {
        let feature = hole(|f| {
            if let DesignFeature::Hole { cut, .. } = f {
                *cut = HoleCut::Counterbore {
                    diameter: 11.0,
                    depth: 4.0,
                };
            }
        });
        let drawing = hole_figure(&feature, None).unwrap();
        let (dimensions, _) = parts(&drawing);
        let bore = dimensions.iter().find(|d| d.text == "Ø11").unwrap();
        // The middle of its line, drawn `offset` to the left of the way.
        let (f, t) = (bore.from, bore.to);
        let mid = [(f[0] + t[0]) / 2.0, (f[1] + t[1]) / 2.0 + bore.offset];
        assert_eq!(measure_at(&feature, mid).as_deref(), Some("Bore Ø"));
        assert_eq!(measure_at(&feature, [-1.0e3, -1.0e3]), None);
    }

    /// A drawing keys a field by its label, whatever colon or repeat mark
    /// its widget id carries.
    #[test]
    fn a_widget_id_keys_its_field() {
        assert_eq!(field_key("Depth:"), "Depth");
        assert_eq!(field_key("Bore Ø#2"), "Bore Ø");
    }
}
