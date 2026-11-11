//! Cost of one pointer query on a large sketch: `snap::resolve` (what every
//! drawing tool's cue and click go through) and `snap::hit_test` (what a
//! selection click takes), timed and counted in allocations.
//!
//! `cargo run --release -p wb_sketch --example snap_bench -- [cells]`: a
//! square grid of `cells` × `cells` cells (20 by default), each a closed
//! rectangle sharing its corners, a circle, an arc and an ellipse arc.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use wb_sketch::sketch::{Arc, Circle, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D};
use wb_sketch::snap::{self, SnapContext};

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

fn main() {
    let cells: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(20);
    let sketch = build(cells);
    println!("{cells}×{cells} cells: {} elements", sketch.geometry.len());
    let queries = cursors(cells, 400);
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
    measure("hit_test", &queries, |c| {
        std::hint::black_box(snap::hit_test(&sketch, c, tol));
    });
}
