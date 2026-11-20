//! What the undo history holds and costs for edits of a large sketch: a
//! grid of `cells` × `cells` cells (20 by default, about 7200 elements),
//! each step one point moved, the whole payload written as the bench
//! writes it and the step closed. Prints the heap the history keeps per
//! step against the payload's JSON text, and the time to close a step,
//! undo one and redo one.
//!
//! `cargo run --release -p wb_sketch --example history_bench -- [cells] [steps]`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use core_document::Document;
use core_document::history::OpJournal;
use sketch_model::SketchFeature;
use sketch_model::sketch::{Arc, Circle, Ellipse, GeometryElement, Line, Point, Sketch, Vec2D};

struct Live;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Live {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        LIVE.fetch_add(size as isize - layout.size() as isize, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Live = Live;

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

/// Min, median and max, in milliseconds.
fn spread(mut times: Vec<Duration>) -> (f64, f64, f64) {
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    (
        ms(times[0]),
        ms(times[times.len() / 2]),
        ms(times[times.len() - 1]),
    )
}

/// The first number in `value`, depth first.
fn first_number(value: &mut serde_json::Value) -> Option<&mut serde_json::Value> {
    match value {
        serde_json::Value::Number(_) => Some(value),
        serde_json::Value::Array(items) => items.iter_mut().find_map(first_number),
        serde_json::Value::Object(map) => map.values_mut().find_map(first_number),
        _ => None,
    }
}

fn main() {
    let arg = |n: usize, default: usize| {
        std::env::args()
            .nth(n)
            .and_then(|a| a.parse().ok())
            .unwrap_or(default)
    };
    let (cells, steps) = (arg(1, 20), arg(2, 50));
    let sketch = build(cells);
    let elements = sketch.geometry.len();
    let plane = sketch.plane;
    let feature = SketchFeature::new(sketch, plane);
    let payload = serde_json::to_value(&feature).expect("json");
    let text = serde_json::to_string(&payload).expect("text").len();
    let mut doc = Document::new("bench");
    let id = doc
        .add_feature_in_body(feature, "sketch".into(), None)
        .expect("sketch feature");
    let mut journal = OpJournal::new(steps + 1);
    journal.note(&mut doc);
    sent(&mut doc);

    let mut closes = Vec::new();
    // The document holds one payload before and after: what grows is
    // the history.
    let before = LIVE.load(Ordering::Relaxed);
    for step in 0..steps {
        // Each write a whole new payload, as the bench's store makes one.
        let mut payload = payload.clone();
        *first_number(&mut payload).expect("a number") = serde_json::json!(step as f64 + 0.5);
        doc.update_feature_data(id, payload).expect("update");
        sent(&mut doc);
        let start = Instant::now();
        journal.note(&mut doc);
        closes.push(start.elapsed());
    }
    let held = LIVE.load(Ordering::Relaxed) - before;
    let rounds = steps.min(20);
    let mut undos = Vec::new();
    for _ in 0..rounds {
        let start = Instant::now();
        journal.undo(&mut doc);
        undos.push(start.elapsed());
        sent(&mut doc);
    }
    let mut redos = Vec::new();
    for _ in 0..rounds {
        let start = Instant::now();
        journal.redo(&mut doc);
        redos.push(start.elapsed());
        sent(&mut doc);
    }
    let per_step = held as f64 / steps as f64;
    println!(
        "{elements} elements, JSON text {:.2} MB, {steps} steps",
        text as f64 / 1e6
    );
    println!(
        "history held per step: {:.2} MB ({:.2}x the text)",
        per_step / 1e6,
        per_step / text as f64
    );
    for (label, times) in [("close", closes), ("undo", undos), ("redo", redos)] {
        let (min, mid, max) = spread(times);
        println!("{label:<6} ms: min {min:.2}  median {mid:.2}  max {max:.2}");
    }
}

/// The outbox sent and confirmed, as the server's acknowledgement leaves
/// it.
fn sent(doc: &mut Document) {
    let _ = doc.take_pending_ops();
    doc.confirm_sent_ops();
}
