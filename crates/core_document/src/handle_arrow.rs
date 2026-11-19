//! How a drag handle on a number looks in the view, the same on every
//! bench: an arrow along the handle's path, its filled head where the
//! number stands, pointing the way it grows.

use crate::workbench::{MarkKind, ScreenSpaceMark, ScreenSpaceOverlay};

/// From the head's tip to its base, in points.
pub const HEAD_LENGTH: f32 = 12.0;
/// Across the head's base, in points.
pub const HEAD_WIDTH: f32 = 10.0;
/// The shaft's thickness, in points.
pub const SHAFT_WIDTH: f32 = 2.0;

/// The arrow of a handle whose path runs through `path` on screen, from
/// where its number starts to where it stands, its head pointing along
/// `way` at the path's end; `scale` is the view's pixels per point. The
/// shaft stops under the head, so the head (a mark, drawn over the lines)
/// covers its end. No head when `way` has no length (the path seen end
/// on); no shaft when the path is shorter than the head.
pub fn handle_arrow(
    path: &[[f32; 2]],
    way: [f32; 2],
    color: [f32; 3],
    scale: f32,
) -> (Vec<ScreenSpaceOverlay>, Vec<ScreenSpaceMark>) {
    let scale = scale.max(0.5);
    let Some(&tip) = path.last() else {
        return (Vec::new(), Vec::new());
    };
    let length = HEAD_LENGTH * scale;
    let shaft = trimmed(path, length * 0.5);
    let lines = ScreenSpaceOverlay::polyline(
        shaft.into_iter().map(Some),
        color,
        SHAFT_WIDTH * scale,
        None,
    );
    let norm = way[0].hypot(way[1]);
    let marks = if norm > 1e-6 {
        vec![ScreenSpaceMark {
            pos: tip,
            color,
            alpha: 1.0,
            kind: MarkKind::Arrowhead {
                way: [way[0] / norm, way[1] / norm],
                length,
                width: HEAD_WIDTH * scale,
            },
        }]
    } else {
        Vec::new()
    };
    (lines, marks)
}

/// `path` with `cut` pixels taken off its end; empty when that leaves
/// nothing.
fn trimmed(path: &[[f32; 2]], cut: f32) -> Vec<[f32; 2]> {
    let mut left = cut;
    let mut points = path.to_vec();
    while points.len() >= 2 {
        let b = points[points.len() - 1];
        let a = points[points.len() - 2];
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        if d > left {
            let t = (d - left) / d;
            let end = points.len() - 1;
            points[end] = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            return points;
        }
        left -= d;
        points.pop();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shaft_stops_under_a_head_at_the_paths_end() {
        let path = [[0.0, 0.0], [30.0, 0.0], [30.0, 40.0]];
        let (lines, marks) = handle_arrow(&path, [0.0, 2.0], [1.0; 3], 1.0);
        let [head] = marks.as_slice() else {
            panic!("one head: {marks:?}")
        };
        assert_eq!(head.pos, [30.0, 40.0]);
        let MarkKind::Arrowhead { way, length, .. } = head.kind else {
            panic!("{:?}", head.kind)
        };
        assert_eq!(way, [0.0, 1.0]);
        let end = lines.last().expect("a shaft").end;
        assert!((end[0] - 30.0).abs() < 1e-4 && (end[1] - (40.0 - length * 0.5)).abs() < 1e-4);
        assert_eq!(lines.len(), 2);

        // Shorter than the head, seen end on: neither shaft nor head.
        let (lines, marks) = handle_arrow(&[[0.0, 0.0], [1.0, 0.0]], [0.0, 0.0], [1.0; 3], 1.0);
        assert!(lines.is_empty() && marks.is_empty());
    }
}
