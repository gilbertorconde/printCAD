//! DXF drawings read by the kernel, handed on as plain 2D polylines
//! (`kernel_api::Drawing2d`).

use kernel_api::{Drawing2d, KernelError, KernelResult};
use ogeom::math::Point2;

/// The curves of DXF text: what the kernel's reader finds, visible and
/// hidden apart, in the drawing's own coordinates.
///
/// # Errors
///
/// `KernelError::Import` when the text is not a DXF.
pub fn read_dxf(text: &str) -> KernelResult<Drawing2d> {
    let drawing = ogeom::io::dxf::read_dxf(text)
        .map_err(|e| KernelError::Import(format!("the DXF could not be read: {e}")))?;
    let plain = |curves: Vec<Vec<Point2>>| -> Vec<Vec<[f64; 2]>> {
        curves
            .into_iter()
            .map(|c| c.into_iter().map(|p| [p.x, p.y]).collect())
            .collect()
    };
    Ok(Drawing2d {
        visible: plain(drawing.visible),
        hidden: plain(drawing.hidden),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_polylines_come_through_by_layer() {
        let text = ogeom::io::dxf::write_dxf(
            &[vec![Point2::new(0.0, 0.0), Point2::new(4.0, 0.0)]],
            &[vec![Point2::new(1.0, 1.0), Point2::new(2.0, 3.5)]],
        );
        let drawing = read_dxf(&text).unwrap();
        assert_eq!(drawing.visible, vec![vec![[0.0, 0.0], [4.0, 0.0]]]);
        assert_eq!(drawing.hidden, vec![vec![[1.0, 1.0], [2.0, 3.5]]]);
    }

    #[test]
    fn text_that_is_not_pairs_is_refused() {
        assert!(read_dxf("0\nSECTION\n2").is_err());
    }
}
