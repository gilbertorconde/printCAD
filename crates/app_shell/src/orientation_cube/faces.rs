//! The cube's face textures, rasterized from the face template once per
//! label and colour, and the colours faces and axes take.

use super::*;

pub(super) fn auto_text_color(bg: Color32) -> Color32 {
    let r = bg.r() as f32 / 255.0;
    let g = bg.g() as f32 / 255.0;
    let b = bg.b() as f32 / 255.0;
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luminance > 0.6 {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(240, 240, 240)
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct FaceKey {
    label: &'static str,
    background: [u8; 3],
    text: [u8; 3],
}

impl FaceKey {
    fn new(label: &'static str, background: Color32, text: Color32) -> Self {
        Self {
            label,
            background: [background.r(), background.g(), background.b()],
            text: [text.r(), text.g(), text.b()],
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct FaceTextureCache {
    handles: HashMap<FaceKey, TextureHandle>,
}

pub(super) fn get_face_texture(
    ctx: &Context,
    label: &'static str,
    background: Color32,
    text: Color32,
) -> Option<TextureHandle> {
    let key = FaceKey::new(label, background, text);
    let cache_id = Id::new("orientation_cube_face_textures");

    if let Some(handle) = ctx.data(|data| {
        data.get_temp::<FaceTextureCache>(cache_id)
            .and_then(|cache| cache.handles.get(&key).cloned())
    }) {
        return Some(handle);
    }

    let texture = create_face_texture(ctx, &key)?;

    ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_insert_with(cache_id, FaceTextureCache::default);
        cache.handles.insert(key.clone(), texture.clone());
    });

    Some(texture)
}

pub(super) fn create_face_texture(ctx: &Context, key: &FaceKey) -> Option<TextureHandle> {
    let svg = FACE_TEMPLATE_SVG
        .replace("{{BACKGROUND_COLOR}}", &rgb_to_hex(key.background))
        .replace("{{TEXT_COLOR}}", &rgb_to_hex(key.text))
        .replace("{{LABEL}}", key.label);
    let image = rasterize_svg(&svg)?;
    let name = format!(
        "orientation_cube_face_{}_{}_{}",
        key.label,
        rgb_to_hex(key.background),
        rgb_to_hex(key.text)
    );
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

pub(super) fn rgb_to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// Pre-render the face textures so the first cube draw doesn't block on SVG
/// rasterization. Which axis a face gets depends on the preset, so every
/// label is warmed in all three axis tints; a preset change then costs
/// nothing.
pub fn warm_face_textures(ctx: &Context) {
    for label in ["FRONT", "REAR", "RIGHT", "LEFT", "TOP", "BOTTOM"] {
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let color = face_tint(axis_color(axis));
            let _ = get_face_texture(ctx, label, color, auto_text_color(color));
        }
    }
}

/// The colour of a cube face whose canonical normal is `canonical`: the
/// triad's colour for the world axis it faces, calmed down to a face fill.
pub(super) fn face_color(axes: &AxisSystem, canonical: Vec3) -> Color32 {
    face_tint(axis_color(axes.canonical_to_world(canonical)))
}

/// The triad's colour for the world axis a direction runs along.
pub(super) fn axis_color(world: Vec3) -> Color32 {
    let a = world.abs();
    if a.x >= a.y && a.x >= a.z {
        ui_kit::tokens::AXIS_X
    } else if a.y >= a.z {
        ui_kit::tokens::AXIS_Y
    } else {
        ui_kit::tokens::AXIS_Z
    }
}

/// An axis colour as a face fill: mixed toward the cube's grey so six of
/// them side by side stay a cube rather than a beach ball, while each still
/// reads unmistakably as its axis.
pub(super) fn face_tint(axis: Color32) -> Color32 {
    const BASE: Color32 = Color32::from_rgb(112, 118, 132);
    const AXIS_SHARE: f32 = 0.55;
    let mix = |a: u8, b: u8| (a as f32 * AXIS_SHARE + b as f32 * (1.0 - AXIS_SHARE)) as u8;
    Color32::from_rgb(
        mix(axis.r(), BASE.r()),
        mix(axis.g(), BASE.g()),
        mix(axis.b(), BASE.b()),
    )
}

pub(crate) fn rasterize_svg(svg: &str) -> Option<ColorImage> {
    let mut fontdb = fontdb::Database::new();
    fontdb.load_system_fonts();
    let opt = Options {
        font_family: "DejaVu Sans".into(),
        languages: vec!["en".into()],
        font_size: 44.0,
        fontdb: std::sync::Arc::new(fontdb),
        ..Options::default()
    };
    let tree = usvg::Tree::from_data(svg.as_bytes(), &opt).ok()?;
    let size = tree.size().to_int_size();
    let (width, height) = (size.width(), size.height());
    let mut pixmap = Pixmap::new(width, height)?;
    let mut pixmap_mut = pixmap.as_mut();
    render(&tree, tiny_skia::Transform::identity(), &mut pixmap_mut);
    let data = pixmap.data().to_vec();
    Some(ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        &data,
    ))
}
