//! Cost of one pointer query on a large sketch: `snap::resolve` (what every
//! drawing tool's cue and click go through) and `snap::hit_test` (what a
//! selection click takes), timed and counted in allocations; then a hover
//! through the bench as the host drives it, a pointer move and the frame's
//! view.
//!
//! `cargo run --release -p wb_sketch --example snap_bench -- [cells] [queries]`:
//! a square grid of `cells` × `cells` cells (20 by default), each a closed
//! rectangle sharing its corners, a circle, an arc and an ellipse arc,
//! queried at `queries` cursors (400 by default).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use core_document::{
    Document, MouseButton, Workbench, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use glam::{Mat4, Vec3};
use wb_sketch::sketch::{Arc, Circle, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D};
use wb_sketch::snap::{self, Guide, SnapContext};
use wb_sketch::{SketchFeature, SketchWorkbench};

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

fn measure(label: &str, queries: &[Vec2D], mut run: impl FnMut(Vec2D)) {
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

/// A pointer move with `tool` in hand over the edited `sketch`, and the
/// view the host then asks for, with the view centred on the grid; with
/// `from`, after a first click there.
fn hover(label: &str, sketch: &Sketch, span: f64, queries: &[Vec2D], tool: &str, from: bool) {
    const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);
    let mid = (span * 0.5) as f32;
    let cam = [mid, mid, span as f32];
    let proj =
        glam::camera::rh::proj::directx::perspective(60f32.to_radians(), 800.0 / 600.0, 0.1, 1e4);
    let view =
        glam::camera::rh::view::look_at_mat4(Vec3::from(cam), Vec3::new(mid, mid, 0.0), Vec3::Y);
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
    let mut wb = SketchWorkbench::default();
    let mut send = |doc: &mut Document, event: Option<WorkbenchInputEvent>| {
        let mut ctx = WorkbenchRuntimeContext::new(doc, cam, [mid, mid, 0.0], VIEWPORT);
        ctx.view_proj = Some(vp);
        ctx.active_document_object = Some(id);
        ctx.selected_body_id = Some(uuid::Uuid::new_v4());
        if let Some(event) = event {
            wb.on_input(&event, Some(tool), &mut ctx);
        }
        wb.on_frame(0.016, &mut ctx);
        std::hint::black_box(wb.viewport(&ctx));
    };
    let px = |doc: &mut Document, p: Vec2D| {
        let mut ctx = WorkbenchRuntimeContext::new(doc, cam, [mid, mid, 0.0], VIEWPORT);
        ctx.view_proj = Some(vp);
        ctx.world_to_viewport([p.x as f32, p.y as f32, 0.0])
            .expect("on screen")
    };
    send(&mut doc, Some(WorkbenchInputEvent::ToolActivated));
    if from {
        let viewport_pos = px(&mut doc, Vec2D::new(span * 0.31, span * 0.47));
        let button = MouseButton::Left;
        send(
            &mut doc,
            Some(WorkbenchInputEvent::MousePress {
                button,
                viewport_pos,
            }),
        );
        send(
            &mut doc,
            Some(WorkbenchInputEvent::MouseRelease {
                button,
                viewport_pos,
            }),
        );
    }
    let moves: Vec<_> = queries.iter().map(|&c| px(&mut doc, c)).collect();
    let mut k = 0;
    measure(label, queries, |_| {
        let viewport_pos = moves[k % moves.len()];
        k += 1;
        send(
            &mut doc,
            Some(WorkbenchInputEvent::MouseMove { viewport_pos }),
        );
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
}
