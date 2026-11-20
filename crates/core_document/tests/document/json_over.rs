//! A value written over another JSON value is exactly what
//! `serde_json::to_value` makes of it, whatever the value it was written
//! over held.

use std::collections::{BTreeMap, HashMap};

use core_document::json_over::write_over;
use serde::Serialize;

#[derive(Serialize)]
enum Shape {
    Unit,
    Newtype(f64),
    Tuple(i32, String),
    Struct {
        id: uuid::Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(tag = "kind")]
enum Tagged {
    Round { r: f32 },
    Square { side: u8, tags: Vec<char> },
}

/// Bytes as serde writes `&[u8]`, which JSON writes as an array.
struct Bytes(Vec<u8>);

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

#[derive(Serialize)]
struct Record {
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    shapes: Vec<Shape>,
    by_number: BTreeMap<u32, bool>,
    by_name: HashMap<String, Vec<u8>>,
    tagged: Option<Tagged>,
    #[serde(flatten)]
    extra: BTreeMap<String, i128>,
    pair: (f64, f64),
    bytes: Bytes,
}

/// Records of every shape serde writes, differing in each: variants,
/// skipped fields, map keys, lengths.
fn records() -> Vec<Record> {
    let id = uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0);
    let record = |name: &str, shapes, tagged| Record {
        name: name.into(),
        shapes,
        by_number: BTreeMap::new(),
        by_name: HashMap::new(),
        tagged,
        extra: BTreeMap::new(),
        pair: (0.0, 0.0),
        bytes: Bytes(Vec::new()),
    };
    let full = Record {
        by_number: [(1, true), (20, false)].into(),
        by_name: [("a".to_string(), vec![1, 2]), ("b".to_string(), vec![])].into(),
        extra: [("big".to_string(), -(1 << 40))].into(),
        pair: (1.5, -0.25),
        bytes: Bytes(vec![0, 255]),
        ..record(
            "full",
            vec![
                Shape::Unit,
                Shape::Newtype(f64::NAN),
                Shape::Tuple(-3, "t".into()),
                Shape::Struct {
                    id,
                    label: Some("l".into()),
                },
            ],
            Some(Tagged::Square {
                side: 4,
                tags: vec!['x', 'é'],
            }),
        )
    };
    let lean = Record {
        by_name: [("b".to_string(), vec![9])].into(),
        ..record(
            "lean",
            vec![Shape::Struct { id, label: None }, Shape::Newtype(2.0)],
            Some(Tagged::Round { r: 0.1 }),
        )
    };
    vec![full, lean, record("none", Vec::new(), None)]
}

/// Every record written over every other (and over a scalar, an array
/// and an empty object) reads as it does made afresh, down to its text.
#[test]
fn a_value_written_over_another_is_the_value_made_afresh() {
    let records = records();
    let afresh: Vec<_> = (records.iter())
        .map(|r| serde_json::to_value(r).unwrap())
        .collect();
    let mut starts = afresh.clone();
    starts.extend([
        serde_json::json!(7),
        serde_json::json!([1, 2, 3]),
        serde_json::json!({"name": [], "stale": true}),
    ]);
    for start in &starts {
        for (record, made) in records.iter().zip(&afresh) {
            let mut over = start.clone();
            write_over(&mut over, record).unwrap();
            assert_eq!(over.to_string(), made.to_string());
        }
    }
}

/// A map key the writer does not take, a float, is written by
/// `to_value`, which leaves nothing of the value written over.
#[test]
fn a_value_it_cannot_write_over_is_made_afresh() {
    struct ByFloat(Vec<(f32, u8)>);
    impl Serialize for ByFloat {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = s.serialize_map(Some(self.0.len()))?;
            for (k, v) in &self.0 {
                map.serialize_entry(k, v)?;
            }
            map.end()
        }
    }
    let floats = ByFloat(vec![(1.5, 1), (-2.0, 2)]);
    let mut over = serde_json::json!({"stale": true, "1.5": 9});
    write_over(&mut over, &floats).unwrap();
    assert_eq!(over, serde_json::to_value(&floats).unwrap());
}
