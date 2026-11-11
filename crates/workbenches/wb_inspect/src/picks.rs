//! The clicks of an element in progress, pinned on the view and numbered
//! until the element is made or the tool is put away: the extra seeds of a
//! fit (Shift and a click on the scan) and the point pairs of an alignment
//! by points (a click on the scan, then the same feature on the reference).
//!
//! Every pin is kept in its body's own frame, so a body moved between the
//! clicks and the alignment (an undo, another client) carries its pins
//! along, and they are read where the bodies sit when the alignment is
//! made. The pins belong to their tab: the bench parks them with it.

use core_document::{
    BodyId, CommandArgs, Document, ScreenSpaceLabel, ScreenSpaceMark, WorkbenchRuntimeContext,
};
use serde_json::json;

/// How many pairs an alignment by points needs.
pub const PAIRS_NEEDED: usize = 3;

#[derive(Debug, Default)]
pub struct Picks {
    /// A fit's seeds pinned so far, on a scan, in its own frame.
    seeds: Vec<(BodyId, [f64; 3])>,
    /// The alignment by points is picking.
    pub aligning: bool,
    /// Its points on the scan and on the reference, each in its body's
    /// own frame.
    on_scan: Vec<(BodyId, [f64; 3])>,
    on_reference: Vec<(BodyId, [f64; 3])>,
}

impl Picks {
    /// Forget every click, as the element is made or the tool put away.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Whether anything is pinned.
    pub fn any(&self) -> bool {
        !self.seeds.is_empty() || !self.on_scan.is_empty() || !self.on_reference.is_empty()
    }

    /// Pin a fit's seed at `at` on `scan`, in the scan's frame. A seed on
    /// another scan starts the pins again.
    pub fn pin_seed(&mut self, scan: BodyId, at: [f64; 3]) {
        self.seeds.retain(|(s, _)| *s == scan);
        self.seeds.push((scan, at));
    }

    /// The seeds pinned on `scan`, taken: the fit they start is made.
    pub fn take_seeds(&mut self, scan: BodyId) -> Vec<[f64; 3]> {
        let seeds = std::mem::take(&mut self.seeds);
        seeds
            .into_iter()
            .filter(|(s, _)| *s == scan)
            .map(|(_, at)| at)
            .collect()
    }

    /// A click of the alignment at `at` (in `body`'s own frame) on `body`,
    /// a scan or the reference: each side takes the next point of its
    /// pairs, or moves the one waiting for its partner.
    pub fn pick_pair_point(&mut self, body: BodyId, scan: bool, at: [f64; 3]) {
        let (side, other) = if scan {
            (&mut self.on_scan, &self.on_reference)
        } else {
            (&mut self.on_reference, &self.on_scan)
        };
        if side.len() > other.len() {
            side.pop();
        }
        side.push((body, at));
    }

    /// How many pairs are complete.
    pub fn pairs(&self) -> usize {
        self.on_scan.len().min(self.on_reference.len())
    }

    /// `inspect.align_points`' arguments for the complete pairs, once
    /// there are enough of them on one scan, every point placed in the
    /// world where its body sits in `document` now; `None` while a picked
    /// body is gone.
    pub fn align_args(&self, document: &Document) -> Option<CommandArgs> {
        let n = self.pairs();
        let scan = self.on_scan.first()?.0;
        if n < PAIRS_NEEDED || self.on_scan.iter().any(|(s, _)| *s != scan) {
            return None;
        }
        let xyz = |&(body, p): &(BodyId, [f64; 3])| {
            let p = world(document, body, p)?;
            Some(json!({"x": p[0], "y": p[1], "z": p[2]}))
        };
        let pairs = self
            .on_scan
            .iter()
            .zip(&self.on_reference)
            .map(|(s, r)| Some(json!({"scan": xyz(s)?, "reference": xyz(r)?})))
            .collect::<Option<Vec<_>>>()?;
        let mut args = CommandArgs::new();
        args.insert("scan".into(), json!(scan.0.to_string()));
        args.insert("pairs".into(), json!(pairs));
        Some(args)
    }

    /// The pins: a fit's seeds numbered in the order clicked, an
    /// alignment's points by their pair, the reference's in its own colour
    /// and in the split view's side pane when it is drawn there.
    pub fn drawn(
        &self,
        ctx: &WorkbenchRuntimeContext,
    ) -> (Vec<ScreenSpaceMark>, Vec<ScreenSpaceLabel>) {
        let palette = &ctx.sketch_palette;
        let document = &*ctx.document;
        let on_view = |body: BodyId, world: [f64; 3]| {
            ctx.world_to_viewport_on(Some(body.0), world.map(|c| c as f32))
                .map(|(x, y)| [x, y])
        };
        let numbered = |points: &[(BodyId, [f64; 3])]| {
            points
                .iter()
                .enumerate()
                .filter_map(|(i, (body, at))| {
                    Some((on_view(*body, world(document, *body, *at)?)?, i + 1))
                })
                .collect::<Vec<_>>()
        };
        let ppp = ctx.pixels_per_point;
        let mut drawn = core_document::numbered_pins(numbered(&self.seeds), palette.selected, ppp);
        for (points, color) in [
            (&self.on_scan, palette.selected),
            (&self.on_reference, palette.external),
        ] {
            let (marks, labels) = core_document::numbered_pins(numbered(points), color, ppp);
            drawn.0.extend(marks);
            drawn.1.extend(labels);
        }
        drawn
    }
}

/// A point in `body`'s own frame placed where the body sits now, while
/// the body is there.
fn world(document: &Document, body: BodyId, at: [f64; 3]) -> Option<[f64; 3]> {
    document.body(body)?;
    Some(
        document
            .body_placement(body)
            .point(at.map(|c| c as f32))
            .map(f64::from),
    )
}
