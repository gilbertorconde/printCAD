//! The view's grid: how strongly each decade of lines draws for its
//! spacing on screen, which decades the renderer is handed, and which one
//! a click snaps to. The renderer fades lines by `FADE_PX`, so the snap
//! reads the same numbers the lines are drawn by.

/// How a decade of lines fades in by its spacing on screen, in pixels:
/// the finest lines from nothing at `[0]` to their full strength at
/// `[1]`, then up to the strength of lines far apart from `[2]` to `[3]`.
pub const FADE_PX: [f32; 4] = [3.0, 14.0, 40.0, 160.0];

/// The share of its full strength a decade has before a click snaps to
/// it: half way through its fade in.
const CLEAR: f32 = 0.5;

/// A grid a bench draws on a plane of its own, in world space, in place of
/// the scene's grid while it is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneGrid {
    /// Where the plane's axes cross: the lines numbered zero.
    pub origin: [f32; 3],
    /// Unit directions in the plane, square to each other; lines run
    /// along both.
    pub x: [f32; 3],
    pub y: [f32; 3],
    /// The spacing the decades are counted from: lines lie at whole
    /// multiples of `base` times a power of ten.
    pub base: f32,
    pub color: [f32; 3],
    /// How opaque lines far apart are; the finest are fainter.
    pub opacity: f32,
    /// The lines' pattern along themselves, in pixels: a period and how
    /// much of it is drawn. A period of zero draws them solid.
    pub dash: [f32; 2],
}

/// How far a decade `px` pixels apart on screen has faded in, 0 to 1.
pub fn fade_in(px: f32) -> f32 {
    let t = ((px - FADE_PX[0]) / (FADE_PX[1] - FADE_PX[0])).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The finest decade drawn at `world_per_px`: `base` times the power of
/// ten that sets lines between `FADE_PX[0]` and ten times it apart, the
/// first spacing that shows at all.
pub fn finest_step(base: f64, world_per_px: f64) -> f64 {
    let base = base.max(1e-9);
    base * 10f64.powf((f64::from(FADE_PX[0]) * world_per_px / base).log10().ceil())
}

/// The finest decade drawn clearly at `world_per_px`, the one a click
/// snaps to: lines one can see.
pub fn clear_step(base: f64, world_per_px: f64) -> f64 {
    let step = finest_step(base, world_per_px);
    if fade_in((step / world_per_px) as f32) >= CLEAR {
        step
    } else {
        step * 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_snaps_to_the_finest_decade_drawn_clearly() {
        for base in [1.0, 7.5, 0.25] {
            for world_per_px in [0.0007, 0.01, 0.033, 0.34, 2.0, 99.0] {
                let finest = finest_step(base, world_per_px);
                let decades = (finest / base).log10();
                assert!((decades - decades.round()).abs() < 1e-9, "{base}: {finest}");
                let px = finest / world_per_px;
                assert!((3.0..30.0).contains(&px), "{base} {world_per_px}: {px} px");
                let clear = clear_step(base, world_per_px);
                assert!(fade_in((clear / world_per_px) as f32) >= CLEAR);
                assert!(clear == finest || fade_in(px as f32) < CLEAR);
            }
        }
        // A tenth of a unit a pixel: lines 1 apart stand 10 px apart, past
        // half their fade; at a fifth they stand 5 px apart and do not.
        assert_eq!(clear_step(1.0, 0.1), 1.0);
        assert_eq!(clear_step(1.0, 0.2), 10.0);
        assert_eq!(clear_step(7.5, 0.5), 7.5);
        assert_eq!(clear_step(7.5, 1.0), 75.0);
    }
}
