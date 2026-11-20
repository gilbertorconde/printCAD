//! Cost of one pointer query on a large sketch: `snap::resolve` (what every
//! drawing tool's cue and click go through) and `snap::hit_test` (what a
//! selection click takes), timed and counted in allocations; then a hover
//! through the bench as the host drives it, a pointer move and the frame's
//! view.
//!
//! Then, on the grid with a dimension on each cell's bottom side and
//! circle, an edit (a segment drawn, the task panel drawn too) and a
//! dimension's label dragged, with and without the panel; and an edit
//! with dimensions on 25 cells only, which the diagnosis analyses; one
//! with a line tangent to each circle, its end held on it, and one with
//! each cell's bottom side fitted to measured points; a point of the
//! plain grid dragged, a solve and a store per frame; and
//! the points the grid leaves free selected, then the pointer moved
//! about with them selected and the panel drawn.
//!
//! `cargo run --release -p wb_sketch --example snap_bench -- [cells] [queries] [only]`:
//! a square grid of `cells` × `cells` cells (20 by default), each a closed
//! rectangle sharing its corners, a circle, an arc and an ellipse arc,
//! queried at `queries` cursors (400 by default; at most 100 edits), the
//! rows timed those whose name holds `only` (all by default).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bench_panel::BenchUi;
use core_document::{
    Document, MouseButton, TaskRequest, Workbench, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use glam::{Mat4, Vec3};
use sketch_model::SketchFeature;
use sketch_model::sketch::{
    Arc, Circle, ConstraintKind, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D,
};
use wb_sketch::SketchWorkbench;
use wb_sketch::snap::{self, Guide, SnapContext};

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const CELL: f64 = 10.0;

fn point(sketch: &mut Sketch, x: f64, y: f64) -> uuid::Uuid {
    sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
}

fn build(cells: usize) -> Sketch {
    let mut sketch = Sketch::new("bench");
    for i in 0..cells {
        for j in 0..cells {
            let (x, y) = (i as f64 * CELL, j as f64 * CELL);
            let corners = [
                point(&mut sketch, x + 1.0, y + 1.0),
                point(&mut sketch, x + 6.0, y + 1.0),
                point(&mut sketch, x + 6.0, y + 4.0),
                point(&mut sketch, x + 1.0, y + 4.0),
            ];
            for k in 0..4 {
                let line = Line::new(corners[k], corners[(k + 1) % 4]);
                sketch.add_geometry(GeometryElement::Line(line));
            }
            let c = point(&mut sketch, x + 8.0, y + 2.5);
            sketch.add_geometry(GeometryElement::Circle(Circle::new(c, 1.2)));
            let (ac, a0, a1) = (
                point(&mut sketch, x + 3.5, y + 7.0),
                point(&mut sketch, x + 5.5, y + 7.0),
                point(&mut sketch, x + 1.5, y + 7.0),
            );
            sketch.add_geometry(GeometryElement::Arc(Arc::new(ac, a0, a1, 2.0)));
            let (ec, e0, e1) = (
                point(&mut sketch, x + 8.0, y + 7.5),
                point(&mut sketch, x + 9.5, y + 7.5),
                point(&mut sketch, x + 6.5, y + 7.5),
            );
            let ellipse = Ellipse::new_arc(ec, Vec2D::new(1.5, 0.0), 0.5, e0, e1);
            sketch.add_geometry(GeometryElement::Ellipse(ellipse));
        }
    }
    sketch
}

/// Cursors spread over the grid, on and between its curves.
fn cursors(cells: usize, n: usize) -> Vec<Vec2D> {
    let span = cells as f64 * CELL;
    (0..n)
        .map(|k| {
            let t = k as f64 * 0.618_033_988_75;
            Vec2D::new(t.fract() * span, (k as f64 / n as f64) * span)
        })
        .collect()
}

/// Whether the row `label` is to be timed.
fn wanted(label: &str) -> bool {
    std::env::args()
        .nth(3)
        .is_none_or(|only| label.contains(&only))
}

fn measure(label: &str, queries: &[Vec2D], mut run: impl FnMut(Vec2D)) {
    if !wanted(label) {
        return;
    }
    for &c in queries.iter().take(8) {
        run(c);
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let start = Instant::now();
    for &c in queries {
        run(c);
    }
    let elapsed = start.elapsed();
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
    let n = queries.len() as f64;
    println!(
        "{label:<28} {:>10.1} µs/query {:>8.1} allocations/query",
        elapsed.as_secs_f64() * 1e6 / n,
        allocations as f64 / n
    );
}

/// The edited sketch in a document, driven through the bench as the host
/// drives it with the view centred on the grid: each step an input
/// event, the frame, the view and the task's header, and with `panel`
/// the task panel drawn as well.
struct Session {
    doc: Document,
    wb: SketchWorkbench,
    id: core_document::FeatureId,
    cam: [f32; 3],
    target: [f32; 3],
    vp: [[f32; 4]; 4],
    panel: Option<egui::Context>,
}

impl Session {
    const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);

    fn new(sketch: &Sketch, span: f64, panel: bool) -> Self {
        let mid = (span * 0.5) as f32;
        let cam = [mid, mid, span as f32];
        let proj = glam::camera::rh::proj::directx::perspective(
            60f32.to_radians(),
            800.0 / 600.0,
            0.1,
            1e4,
        );
        let view = glam::camera::rh::view::look_at_mat4(
            Vec3::from(cam),
            Vec3::new(mid, mid, 0.0),
            Vec3::Y,
        );
        let vp = (Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * proj * view).to_cols_array_2d();
        let mut doc = Document::new("bench");
        let plane = sketch.plane;
        let id = doc
            .add_feature_in_body(
                SketchFeature::new(sketch.clone(), plane),
                "sketch".into(),
                None,
            )
            .expect("sketch feature");
        Self {
            doc,
            wb: SketchWorkbench::default(),
            id,
            cam,
            target: [mid, mid, 0.0],
            vp,
            panel: panel.then(|| {
                let egui = egui::Context::default();
                ui_kit::theme::apply_theme(&egui);
                egui
            }),
        }
    }

    /// The bench and the context a hook of it is handed this frame.
    fn split(&mut self) -> (&mut SketchWorkbench, WorkbenchRuntimeContext<'_>) {
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut self.doc, self.cam, self.target, Self::VIEWPORT);
        ctx.view_proj = Some(self.vp);
        ctx.active_document_object = Some(self.id);
        ctx.selected_body_id = Some(uuid::Uuid::nil());
        (&mut self.wb, ctx)
    }

    fn send(&mut self, event: Option<WorkbenchInputEvent>, tool: &str) {
        let panel = self.panel.clone();
        let (wb, mut ctx) = self.split();
        if let Some(event) = event {
            wb.on_input(&event, Some(tool), &mut ctx);
        }
        wb.on_frame(0.016, &mut ctx);
        std::hint::black_box(wb.viewport(&ctx));
        std::hint::black_box(wb.task(&ctx));
        if let Some(egui) = panel {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 900.0),
                )),
                ..Default::default()
            };
            let _ = egui.run_ui(raw, |ui| {
                wb.ui_task_panel(ui, &mut ctx, TaskRequest::default());
            });
        }
    }

    fn px(&mut self, p: Vec2D) -> (f32, f32) {
        self.split()
            .1
            .world_to_viewport([p.x as f32, p.y as f32, 0.0])
            .expect("on screen")
    }

    fn button(&mut self, press: bool, viewport_pos: (f32, f32), tool: &str) {
        let button = MouseButton::Left;
        let event = if press {
            WorkbenchInputEvent::MousePress {
                button,
                viewport_pos,
            }
        } else {
            WorkbenchInputEvent::MouseRelease {
                button,
                viewport_pos,
            }
        };
        self.send(Some(event), tool);
    }

    fn click(&mut self, viewport_pos: (f32, f32), tool: &str) {
        self.button(true, viewport_pos, tool);
        self.button(false, viewport_pos, tool);
    }

    fn move_to(&mut self, viewport_pos: (f32, f32), tool: &str) {
        self.send(Some(WorkbenchInputEvent::MouseMove { viewport_pos }), tool);
    }
}

/// A pointer move with `tool` in hand over the edited `sketch`; with
/// `from`, after a first click there.
fn hover(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D], tool: &str, from: bool) {
    if !wanted(label) {
        return;
    }
    let mut s = Session::new(sketch, span, false);
    s.send(Some(WorkbenchInputEvent::ToolActivated), tool);
    if from {
        let at = s.px(Vec2D::new(span * 0.31, span * 0.47));
        s.click(at, tool);
    }
    let moves: Vec<_> = queries.iter().map(|&c| s.px(c)).collect();
    let mut k = 0;
    measure(label, queries, |_| {
        s.move_to(moves[k % moves.len()], tool);
        k += 1;
    });
}

/// The first `cells` cells' bottom sides held to their length and their
/// circles to their radius: two dimension labels per cell.
fn dimensioned(mut sketch: Sketch, cells: usize) -> Sketch {
    let lines: Vec<_> = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Line(_)))
        .map(|g| g.id())
        .step_by(4)
        .take(cells)
        .collect();
    let circles: Vec<_> = sketch
        .geometry
        .iter()
        .filter(|g| matches!(g, GeometryElement::Circle(_)))
        .map(|g| g.id())
        .take(cells)
        .collect();
    for line in lines {
        sketch.add_constraint(ConstraintKind::Length { line, length: 5.0 });
    }
    for circle in circles {
        sketch.add_constraint(ConstraintKind::Radius {
            circle,
            radius: 1.2,
        });
    }
    sketch
}

/// A line touching each cell's circle at its top, its end held on the
/// circle and tangent to it.
fn tangent_lines(mut sketch: Sketch) -> Sketch {
    let circles: Vec<(uuid::Uuid, Vec2D, f64)> = (sketch.geometry.iter())
        .filter_map(|g| match g {
            GeometryElement::Circle(c) => Some((c.id, sketch.point_position(c.center)?, c.radius)),
            _ => None,
        })
        .collect();
    for (circle, centre, r) in circles {
        let p = point(&mut sketch, centre.x, centre.y + r);
        let q = point(&mut sketch, centre.x + 1.5, centre.y + r);
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(p, q)));
        sketch.add_constraint(ConstraintKind::PointOnCircle { point: p, circle });
        sketch.add_constraint(ConstraintKind::Tangent {
            line_or_circle1: line,
            item2: circle,
        });
    }
    sketch
}

/// Each cell's bottom side fitted to measured points along it, as a
/// scan's slice leaves them.
fn fitted(mut sketch: Sketch) -> Sketch {
    let lines: Vec<(uuid::Uuid, Vec2D, Vec2D)> = (sketch.geometry.iter())
        .filter_map(|g| match g {
            GeometryElement::Line(l) => Some((
                l.id,
                sketch.point_position(l.start)?,
                sketch.point_position(l.end)?,
            )),
            _ => None,
        })
        .step_by(4)
        .collect();
    for (element, a, b) in lines {
        let points = [0.25, 0.5, 0.75]
            .map(|t| [a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t + 0.01])
            .to_vec();
        sketch
            .soft
            .push(sketch_model::sketch::SoftData { element, points });
    }
    sketch
}

/// A segment drawn per click with the line tool, each one an edit
/// stored, solved and shown, the task panel drawn too.
fn edit(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D]) {
    if !wanted(label) {
        return;
    }
    let mut s = Session::new(sketch, span, true);
    let tool = "sketch.line";
    s.send(Some(WorkbenchInputEvent::ToolActivated), tool);
    let clicks: Vec<_> = queries
        .iter()
        .map(|&c| s.px(Vec2D::new(c.x + 0.37, c.y + 0.21)))
        .collect();
    s.click(clicks[clicks.len() - 1], tool);
    let mut k = 0;
    measure(label, queries, |_| {
        let at = clicks[k % clicks.len()];
        k += 1;
        s.move_to(at, tool);
        s.click(at, tool);
    });
}

/// A dimension's label dragged about, a pointer move per frame, with
/// the task panel drawn or not.
fn label_drag(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D], panel: bool) {
    if !wanted(label) {
        return;
    }
    let mut s = Session::new(sketch, span, panel);
    let tool = "sketch.select";
    s.send(Some(WorkbenchInputEvent::ToolActivated), tool);
    let labels = {
        let (wb, ctx) = s.split();
        wb.viewport(&ctx).labels
    };
    let from_middle =
        |l: &core_document::ScreenSpaceLabel| (l.pos[0] - 400.0).hypot(l.pos[1] - 300.0);
    let grab = labels
        .iter()
        .filter(|l| l.background)
        .min_by(|a, b| from_middle(a).total_cmp(&from_middle(b)))
        .map(|l| (l.pos[0], l.pos[1]))
        .expect("a dimension label on screen");
    s.button(true, grab, tool);
    let moves: Vec<_> = (0..queries.len())
        .map(|k| {
            let t = k as f32 * 0.37;
            (grab.0 + 30.0 * t.sin() + 12.0, grab.1 + 30.0 * t.cos())
        })
        .collect();
    let mut k = 0;
    measure(label, queries, |_| {
        s.move_to(moves[k % moves.len()], tool);
        k += 1;
    });
    s.button(false, moves[0], tool);
}

/// The end of the middle cell's arc dragged about with the select tool,
/// a pointer move per frame, each one solved and stored as one gesture.
/// The grid has no dimensions here: their labels cover every point at
/// this zoom, and a press on a label drags the label.
fn point_drag(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D]) {
    if !wanted(label) {
        return;
    }
    let mut s = Session::new(sketch, span, false);
    let tool = "sketch.select";
    s.send(Some(WorkbenchInputEvent::ToolActivated), tool);
    let mid = ((span / CELL) as usize / 2) as f64 * CELL;
    let grab = s.px(Vec2D::new(mid + 5.5, mid + 7.0));
    s.move_to(grab, tool);
    s.button(true, grab, tool);
    let moves: Vec<_> = (0..queries.len())
        .map(|k| {
            let t = k as f32 * 0.37;
            (grab.0 + 20.0 * t.sin() + 20.0, grab.1 + 20.0 * t.cos())
        })
        .collect();
    let mut k = 0;
    measure(label, queries, |_| {
        s.move_to(moves[k % moves.len()], tool);
        k += 1;
    });
    s.button(false, moves[0], tool);
}

/// The points the dimensioned grid leaves free selected, as the Select
/// degrees of freedom action does.
fn select_free(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D]) {
    if !wanted(label) {
        return;
    }
    let mut s = Session::new(sketch, span, false);
    measure(label, queries, |_| {
        s.send(
            Some(WorkbenchInputEvent::ToolActivated),
            "sketch.select_dof",
        );
    });
}

/// Pointer moves with the select tool, the panel drawn, while the free
/// points are selected.
fn hover_selected(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D]) {
    if !wanted(label) {
        return;
    }
    let mut s = Session::new(sketch, span, true);
    s.send(
        Some(WorkbenchInputEvent::ToolActivated),
        "sketch.select_dof",
    );
    let tool = "sketch.select";
    s.send(Some(WorkbenchInputEvent::ToolActivated), tool);
    let moves: Vec<_> = queries.iter().map(|&c| s.px(c)).collect();
    let mut k = 0;
    measure(label, queries, |_| {
        s.move_to(moves[k % moves.len()], tool);
        k += 1;
    });
}

fn main() {
    let cells: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(20);
    let sketch = build(cells);
    println!("{cells}×{cells} cells: {} elements", sketch.geometry.len());
    let count: usize = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(400);
    let queries = cursors(cells, count);
    let tol = 0.4;
    let free = SnapContext::default();
    measure("resolve, no segment", &queries, |c| {
        std::hint::black_box(snap::resolve(&sketch, c, tol, &free));
    });
    let arcs = SnapContext {
        arcs: true,
        ..SnapContext::default()
    };
    measure("resolve, arc helpers", &queries, |c| {
        std::hint::black_box(snap::resolve(&sketch, c, tol, &arcs));
    });
    measure("resolve, drawing a segment", &queries, |c| {
        let from = SnapContext {
            from: Some(Vec2D::new(c.x - 7.3, c.y - 3.1)),
            ..SnapContext::default()
        };
        std::hint::black_box(snap::resolve(&sketch, c, tol, &from));
    });
    // The circle a centre-point arc's end lands on, passing just off the
    // cursor.
    measure("resolve, placing an arc's end", &queries, |c| {
        let centre = Vec2D::new(c.x + 3.1, c.y + 2.2);
        let r = (c - centre).to_glam().length() + 0.1;
        let end = SnapContext {
            guide: Some(Guide::Circle { centre, r }),
            ..SnapContext::default()
        };
        std::hint::black_box(snap::resolve(&sketch, c, tol, &end));
    });
    // Likewise the ellipse an elliptic arc's end lands on, whose crossings
    // are found numerically.
    measure("resolve, an ellipse arc's end", &queries, |c| {
        let major = Vec2D::new(4.0, 1.5);
        let centre = Vec2D::new(c.x + 3.1, c.y + 2.2);
        let guide = Guide::Ellipse {
            centre,
            major,
            ratio: 0.5,
        };
        let on = guide.foot(c).unwrap_or(c);
        let end = SnapContext {
            guide: Some(guide),
            ..SnapContext::default()
        };
        let c = Vec2D::new(on.x + 0.1, on.y);
        std::hint::black_box(snap::resolve(&sketch, c, tol, &end));
    });
    measure("hit_test", &queries, |c| {
        std::hint::black_box(snap::hit_test(&sketch, c, tol));
    });
    let span = cells as f64 * CELL;
    hover(
        "hover, line tool",
        &sketch,
        span,
        &queries,
        "sketch.line",
        false,
    );
    hover(
        "hover, drawing a line",
        &sketch,
        span,
        &queries,
        "sketch.line",
        true,
    );
    hover(
        "hover, drawing a rectangle",
        &sketch,
        span,
        &queries,
        "sketch.rect",
        true,
    );
    let marked = dimensioned(sketch.clone(), usize::MAX);
    let edits = &queries[..queries.len().min(100)];
    edit("edit, a segment drawn", &marked, span, edits);
    // Few enough dimensions for the diagnosis to look for redundancy.
    let few = dimensioned(sketch.clone(), 25);
    edit("edit, 50 dimensions", &few, span, edits);
    edit(
        "edit, tangent lines",
        &tangent_lines(marked.clone()),
        span,
        edits,
    );
    edit("edit, fitted sides", &fitted(marked.clone()), span, edits);
    point_drag("point drag", &sketch, span, edits);
    select_free(
        "select free points",
        &marked,
        span,
        &queries[..queries.len().min(4)],
    );
    hover_selected("hover, free points selected", &marked, span, &queries);
    label_drag("label drag", &marked, span, &queries, false);
    label_drag("label drag, panel drawn", &marked, span, &queries, true);
}
