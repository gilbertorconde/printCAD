//! The line-icon set: 24px, 1.5px stroke, painted white and tinted at draw
//! time. Textures rasterize once per name and size, and live in the egui
//! context.
//!
//! The same table also carries the motion drawings, which are larger, keep
//! their own colours, and are drawn through [`drawing`].

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, ColorImage, Context, Id, Image, Response, TextureHandle, TextureOptions, Ui};

use crate::icon_table::ICONS;

/// Rasterization size in pixels; icons draw at 20px or less, so 48px stays
/// crisp on 2× displays.
const RASTER_PX: u32 = 48;

/// The SVG source of a vendored icon.
pub fn svg(name: &str) -> Option<&'static str> {
    ICONS
        .binary_search_by(|(n, _)| n.cmp(&name))
        .ok()
        .map(|i| ICONS[i].1)
}

/// Whether `name` is in the set.
pub fn exists(name: &str) -> bool {
    svg(name).is_some()
}

/// Every vendored icon name, sorted.
pub fn names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, _)| *n)
}

/// A drawing shown at this many points or larger rasterizes at twice its
/// size, so it stays sharp where an icon-sized texture would smear.
const DRAWING_SCALE: f32 = 2.0;

#[derive(Default, Clone)]
struct IconCache {
    handles: HashMap<(String, u32), TextureHandle>,
}

/// The white texture for `name` at the icon set's own size.
pub fn texture(ctx: &Context, name: &str) -> Option<TextureHandle> {
    texture_at(ctx, name, RASTER_PX)
}

/// The texture for `name` rasterized to `px`, on first use. Unknown names
/// log once and yield `None`.
pub fn texture_at(ctx: &Context, name: &str, px: u32) -> Option<TextureHandle> {
    let cache_id = Id::new("ui_kit::icon_cache");
    let key = (name.to_owned(), px);
    if let Some(handle) = ctx.data(|d| {
        d.get_temp::<IconCache>(cache_id)
            .and_then(|c| c.handles.get(&key).cloned())
    }) {
        return Some(handle);
    }
    let Some(source) = svg(name) else {
        let missing_id = Id::new(("ui_kit::icon_missing", name));
        let already = ctx
            .data(|d| d.get_temp::<bool>(missing_id))
            .unwrap_or(false);
        if !already {
            tracing::warn!(target: "printcad.ui", icon = name, "unknown icon name");
            ctx.data_mut(|d| d.insert_temp(missing_id, true));
        }
        return None;
    };
    let image = rasterize(source, px)?;
    let handle = ctx.load_texture(format!("icon::{name}@{px}"), image, TextureOptions::LINEAR);
    ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with(cache_id, IconCache::default)
            .handles
            .insert(key, handle.clone());
    });
    Some(handle)
}

/// An `Image` widget for a drawing at `size` points, in its own colours and
/// rasterized for the size it is shown at.
pub fn drawing(ctx: &Context, name: &str, size: f32) -> Option<Image<'static>> {
    let px = (size * DRAWING_SCALE).round().max(1.0) as u32;
    let handle = texture_at(ctx, name, px)?;
    Some(Image::from_texture(&handle).fit_to_exact_size(egui::vec2(size, size)))
}

/// An `Image` widget for `name` at `size` px, tinted `tint`.
pub fn image(ctx: &Context, name: &str, size: f32, tint: Color32) -> Option<Image<'static>> {
    let handle = texture(ctx, name)?;
    Some(
        Image::from_texture(&handle)
            .fit_to_exact_size(egui::vec2(size, size))
            .tint(tint),
    )
}

/// Draw `name` inline at `size` px in `tint`; draws nothing for an unknown
/// name so layouts never shift.
pub fn draw(ui: &mut Ui, name: &str, size: f32, tint: Color32) -> Response {
    match image(ui.ctx(), name, size, tint) {
        Some(img) => ui.add(img),
        None => ui.allocate_response(egui::vec2(size, size), egui::Sense::hover()),
    }
}

/// Rasterize an SVG to a square `px` image. No font database is attached:
/// the icon set is pure geometry, and scanning system fonts costs tens of
/// milliseconds per call.
pub fn rasterize(source: &str, px: u32) -> Option<ColorImage> {
    let opt = usvg::Options {
        fontdb: Arc::new(usvg::fontdb::Database::new()),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(source.as_bytes(), &opt).ok()?;
    let size = tree.size();
    let scale = px as f32 / size.width().max(size.height());
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(ColorImage::from_rgba_premultiplied(
        [px as usize, px as usize],
        pixmap.data(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_matches_the_icon_directory() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("icons");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .expect("icons dir")
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".svg").map(str::to_owned)
            })
            .collect();
        on_disk.sort();
        let in_table: Vec<String> = names().map(str::to_owned).collect();
        assert_eq!(in_table, on_disk, "run scripts/vendor-icons.mjs");
    }

    #[test]
    fn the_table_is_sorted_for_binary_search() {
        let sorted = ICONS.windows(2).all(|w| w[0].0 < w[1].0);
        assert!(sorted);
        assert!(exists("pad"));
        assert!(!exists("no-such-icon"));
    }

    #[test]
    fn every_icon_rasterizes_with_visible_pixels() {
        for name in names() {
            let image = rasterize(svg(name).unwrap(), 24).expect(name);
            let lit = image.pixels.iter().filter(|p| p.a() > 0).count();
            assert!(lit > 0, "{name} rasterized blank");
        }
    }
}
