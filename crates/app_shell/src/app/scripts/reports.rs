//! Picture and report output adapters.

use core_document::{Args, CommandArgs, CommandError, CommandResult};
use serde_json::json;
use ui_kit::tokens;

/// The design's colours and label size, as the pictures of `doc.picture`
/// and the `view` tool draw in them.
pub(crate) fn picture_ink() -> doc_render::proof::Ink {
    let rgb = |c: egui::Color32| [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
    doc_render::proof::Ink {
        paint: rgb(tokens::WARNING),
        background: (rgb(tokens::VIEWPORT_TOP), rgb(tokens::VIEWPORT_BOTTOM)),
        halo: rgb(tokens::BG0),
        annotation: rgb(tokens::ANNOTATION),
        axes: [tokens::AXIS_X, tokens::AXIS_Y, tokens::AXIS_Z].map(rgb),
        marker: rgb(tokens::DANGER),
        label: rgb(tokens::TEXT1),
        resolved: rgb(tokens::TEXT3),
        asks: rgb(tokens::WARNING),
        note: rgb(tokens::ACCENT),
        font_size: tokens::FONT_MD,
    }
}

/// `doc.picture`: the request in `args` drawn by `draw` and written to
/// its `path`.
pub(crate) fn picture_command(
    args: &CommandArgs,
    draw: impl FnOnce(&doc_render::proof::Request) -> Result<Vec<u8>, String>,
) -> CommandResult {
    let path = std::path::PathBuf::from(Args(args).string("path")?);
    let request = doc_render::proof::parse(args).map_err(CommandError::failed)?;
    let png = draw(&request).map_err(CommandError::failed)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| CommandError::failed(e.to_string()))?;
    }
    std::fs::write(&path, png).map_err(|e| CommandError::failed(e.to_string()))?;
    Ok(json!({
        "path": path.display().to_string(),
        "width": request.size.0,
        "height": request.size.1,
        "renderer": doc_render::proof::RENDERER,
    }))
}

impl crate::PrintCadApp {
    /// `doc.report_pdf`: every bench's report, pictured and tabled, as a
    /// PDF at `path`.
    pub(super) fn report_pdf(&self, args: &CommandArgs) -> CommandResult {
        let a = core_document::Args(args);
        let path = std::path::PathBuf::from(a.string("path")?);
        let document = &self.session.document;
        let mut reports = self.registry.reports(document);
        if let Some(maps) = maps_report(document, &self.session.shown_maps) {
            reports.push(maps);
        }
        if reports.is_empty() {
            return Err(CommandError::failed(
                "no workbench has anything to report yet",
            ));
        }
        let title = a
            .opt_string("title")?
            .map_or_else(|| format!("{} report", document.name()), str::to_string);
        let bytes = report_document(&title, &reports, |bodies, view| {
            let mut request = serde_json::Map::new();
            request.insert("view".into(), json!(view));
            request.insert(
                "bodies".into(),
                json!(bodies.iter().map(|b| b.0.to_string()).collect::<Vec<_>>()),
            );
            request.insert("size".into(), json!([480, 360]));
            let request = doc_render::proof::parse(&request).ok()?;
            let png = self.picture(&request).ok()?;
            decode_png(&png)
        });
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, &bytes.0)
            .map_err(|e| CommandError::failed(format!("{}: {e}", path.display())))?;
        Ok(json!({"path": path.display().to_string(), "pages": bytes.1, "sections": reports.len()}))
    }
}

/// The maps the view shows, as a report's section: each one's scale and
/// readout, and its bodies pictured painted by it.
fn maps_report(
    document: &core_document::Document,
    maps: &[core_document::ScalarMap],
) -> Option<core_document::BenchReport> {
    if maps.is_empty() {
        return None;
    }
    let unit = document.display_unit();
    let mut summary = Vec::new();
    for map in maps {
        let end = |v: f32| {
            if map.legend.unit == "mm" {
                core_document::units::show_mm(f64::from(v), unit)
            } else {
                format!("{v:.3} {}", map.legend.unit)
            }
        };
        summary.push(format!(
            "{}: colours from {} to {}",
            map.legend.title,
            end(map.range[0]),
            end(map.range[1])
        ));
        for (name, value) in &map.legend.stats {
            summary.push(format!("    {name}: {value}"));
        }
    }
    let mut bodies: Vec<_> = maps.iter().map(|m| m.body).collect();
    bodies.dedup();
    Some(core_document::BenchReport {
        title: "Maps".to_string(),
        summary,
        tables: Vec::new(),
        bodies,
    })
}

/// A PNG's pixels as RGBA, with its size.
fn decode_png(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut rgba).ok()?;
    rgba.truncate(frame.buffer_size());
    (frame.color_type == png::ColorType::Rgba && frame.bit_depth == png::BitDepth::Eight)
        .then_some((rgba, frame.width, frame.height))
}

/// The report laid out on A4 pages: its title, then each section's
/// heading, summary, pictures (three views of its bodies, from `draw`)
/// and tables, a new page whenever one fills. The PDF's bytes and pages.
fn report_document(
    title: &str,
    reports: &[core_document::BenchReport],
    draw: impl Fn(&[core_document::BodyId], &str) -> Option<(Vec<u8>, u32, u32)>,
) -> (Vec<u8>, usize) {
    use doc_render::pdf::{HEIGHT, Pdf, WIDTH, width};
    const MARGIN: f32 = 40.0;
    let mut pdf = Pdf::default();
    let mut pages = 1;
    pdf.page();
    let mut y = MARGIN + 16.0;
    pdf.text(MARGIN, y, 16.0, true, title);
    y += 8.0;
    pdf.line(MARGIN, y, WIDTH - MARGIN, y);
    y += 18.0;
    let mut room = |pdf: &mut Pdf, y: &mut f32, need: f32| {
        if *y + need > HEIGHT - MARGIN {
            pdf.page();
            pages += 1;
            *y = MARGIN + 12.0;
        }
    };
    for report in reports {
        room(&mut pdf, &mut y, 40.0);
        pdf.text(MARGIN, y, 13.0, true, &report.title);
        y += 16.0;
        for line in &report.summary {
            pdf.text(MARGIN, y, 10.0, false, line);
            y += 13.0;
        }
        if !report.bodies.is_empty() {
            let gap = 10.0;
            let bw = (WIDTH - 2.0 * MARGIN - 2.0 * gap) / 3.0;
            let bh = bw * 0.75;
            room(&mut pdf, &mut y, bh + 24.0);
            for (k, view) in ["iso", "top", "front"].iter().enumerate() {
                let x = MARGIN + k as f32 * (bw + gap);
                if let Some((rgba, w, h)) = draw(&report.bodies, view) {
                    pdf.picture(&rgba, w, h, x, y, bw, bh);
                }
                pdf.text(x, y + bh + 11.0, 8.0, false, view);
            }
            y += bh + 34.0;
        }
        for table in &report.tables {
            room(&mut pdf, &mut y, 40.0);
            pdf.text(MARGIN, y, 11.0, true, &table.heading);
            y += 15.0;
            // Columns as wide as their widest cell, shrunk together to fit.
            let size = 7.5;
            let n = table.columns.len().max(1);
            let mut widths: Vec<f32> = (0..n)
                .map(|c| {
                    table
                        .rows
                        .iter()
                        .filter_map(|r| r.get(c))
                        .chain(table.columns.get(c))
                        .map(|t| width(t, size))
                        .fold(20.0, f32::max)
                        .min(220.0)
                        + 6.0
                })
                .collect();
            let total: f32 = widths.iter().sum();
            let fit = (WIDTH - 2.0 * MARGIN) / total;
            if fit < 1.0 {
                widths.iter_mut().for_each(|w| *w *= fit);
            }
            let fitted = |text: &str, w: f32| -> String {
                if width(text, size) <= w - 4.0 {
                    return text.to_string();
                }
                let mut out = String::new();
                for c in text.chars() {
                    if width(&format!("{out}{c}…"), size) > w - 4.0 {
                        break;
                    }
                    out.push(c);
                }
                format!("{out}…")
            };
            let row = |pdf: &mut Pdf, y: f32, cells: &[String], bold: bool| {
                let mut x = MARGIN;
                for (c, w) in widths.iter().enumerate() {
                    if let Some(text) = cells.get(c) {
                        pdf.text(x, y, size, bold, &fitted(text, *w));
                    }
                    x += w;
                }
            };
            row(&mut pdf, y, &table.columns, true);
            y += 4.0;
            pdf.line(MARGIN, y, WIDTH - MARGIN, y);
            y += 10.0;
            for cells in &table.rows {
                room(&mut pdf, &mut y, 11.0);
                row(&mut pdf, y, cells, false);
                y += 11.0;
            }
            y += 10.0;
        }
        y += 8.0;
    }
    (pdf.finish(), pages)
}
