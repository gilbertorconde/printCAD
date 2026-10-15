//! The sketcher's integration tests, one program.
//!
//! One program rather than one per file: each would link the whole crate
//! and its dependencies again, which costs far more than running the tests.

mod block_kernel;
mod interaction;
mod operations;
mod precision;

use core_document::KeyCode;
use glam::{Mat4, Vec3};
use wb_sketch::sketch::{self, GeometryElement, Sketch};

/// `$all` lists a sketch's elements of one kind, `$first` takes the first.
macro_rules! kind {
    ($all:ident, $variant:ident $(, $first:ident)?) => {
        fn $all(sketch: &Sketch) -> Vec<sketch::$variant> {
            let of = |g: &GeometryElement| match g {
                GeometryElement::$variant(e) => Some(e.clone()),
                _ => None,
            };
            sketch.geometry.iter().filter_map(of).collect()
        }
        $(fn $first(sketch: &Sketch) -> sketch::$variant {
            $all(sketch)
                .into_iter()
                .next()
                .expect("an element of the kind")
        })?
    };
}
kind!(all_points, Point);
kind!(all_lines, Line, line_of);
kind!(all_arcs, Arc, arc_of);
kind!(all_circles, Circle, circle_of);
kind!(all_ellipses, Ellipse, ellipse_of);
kind!(all_splines, BSpline, spline_of);

/// Whether the sketch holds a constraint of `kind`, named as its variant
/// is, and with `value` given, one dimensioning that (degrees for angles).
fn holds(sketch: &Sketch, kind: &str, value: Option<f64>) -> bool {
    sketch.constraints.iter().any(|c| {
        let named = format!("{:?}", c.kind);
        let dim = sketch::dimension_value(&c.kind);
        named.split([' ', '(']).next() == Some(kind)
            && value.is_none_or(|v| dim.is_some_and(|d| (d - v).abs() < 1e-3))
    })
}

/// The view the tests look through: 800 by 600 pixels, the camera 50 above
/// the XY sketch plane.
const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);
const CAM_POS: [f32; 3] = [0.0, 0.0, 50.0];

/// Vulkan-convention view-projection matching the app camera: perspective
/// with the Y flip baked in, looking straight down +Z at the default XY
/// sketch plane.
fn view_proj() -> [[f32; 4]; 4] {
    let proj = glam::camera::rh::proj::directx::perspective(
        60f32.to_radians(),
        800.0 / 600.0,
        0.1,
        1000.0,
    );
    let flip_y = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
    let view = glam::camera::rh::view::look_at_mat4(Vec3::from_array(CAM_POS), Vec3::ZERO, Vec3::Y);
    (flip_y * proj * view).to_cols_array_2d()
}

/// The character a number key types, as the app sends it after the key.
fn number_text(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::Key0 => "0",
        KeyCode::Key1 => "1",
        KeyCode::Key2 => "2",
        KeyCode::Key3 => "3",
        KeyCode::Key4 => "4",
        KeyCode::Key5 => "5",
        KeyCode::Key6 => "6",
        KeyCode::Key7 => "7",
        KeyCode::Key8 => "8",
        KeyCode::Key9 => "9",
        KeyCode::Period => ".",
        KeyCode::Comma => ",",
        KeyCode::Minus => "-",
        _ => return None,
    })
}
