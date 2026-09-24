//! A number a feature holds, which may be set by a formula.
//!
//! In a feature's JSON a parameter is a plain number, or, when a formula
//! sets it, `{"formula": "3 * Printer.nozzle", "value": 1.2}`: the formula
//! as typed and the value it had when it was. A document without formulas
//! reads and writes exactly as it did before they existed.
//!
//! The document keeps what the user typed. What a formula comes to now is
//! derived, worked out on each replica by [`crate::evaluate`] and kept
//! beside the feature (`Document::feature_values`); a bench builds from
//! that, so `value` is only what the formula last gave when it was set.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

/// A number, or a formula and the value it last gave.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Param<T = f32> {
    pub value: T,
    pub formula: Option<String>,
}

impl<T: Copy> Param<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            formula: None,
        }
    }

    pub fn get(&self) -> T {
        self.value
    }
}

impl<T> From<T> for Param<T> {
    fn from(value: T) -> Self {
        Self {
            value,
            formula: None,
        }
    }
}

impl<T: Serialize> Serialize for Param<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.formula {
            None => self.value.serialize(serializer),
            Some(formula) => {
                #[derive(Serialize)]
                struct Bound<'a, T> {
                    formula: &'a str,
                    value: &'a T,
                }
                Bound {
                    formula,
                    value: &self.value,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de, T: DeserializeOwned> Deserialize<'de> for Param<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let (number, formula) = match &value {
            Value::Object(map) => (
                map.get("value").cloned().unwrap_or(json!(0)),
                map.get("formula")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ),
            other => (other.clone(), None),
        };
        let value = T::deserialize(number).map_err(serde::de::Error::custom)?;
        Ok(Self { value, formula })
    }
}

/// The number at `slot` in a feature's JSON and its formula, if it is a
/// parameter.
pub fn read(slot: &Value) -> Option<(f64, Option<&str>)> {
    match slot {
        Value::Number(n) => Some((n.as_f64()?, None)),
        Value::Object(map) => Some((
            map.get("value").and_then(Value::as_f64)?,
            Some(map.get("formula")?.as_str()?),
        )),
        _ => None,
    }
}

/// Put `value` at `slot`, keeping its formula.
pub fn write_value(slot: &mut Value, value: f64) {
    match slot {
        Value::Object(map) => {
            map.insert("value".to_string(), json!(value));
        }
        other => *other = json!(value),
    }
}

/// Set `slot` to `value` with `formula`, or to the bare number without
/// one.
pub fn set(slot: &mut Value, value: f64, formula: Option<&str>) {
    *slot = match formula {
        Some(formula) => json!({"formula": formula, "value": value}),
        None => json!(value),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Pad {
        length: Param,
        depth: Param<f64>,
    }

    #[test]
    fn a_plain_number_stays_a_plain_number_and_a_formula_keeps_its_value() {
        let old: Pad = serde_json::from_value(json!({"length": 10.0, "depth": 2})).unwrap();
        assert_eq!(old.length, Param::new(10.0));
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            json!({"length": 10.0, "depth": 2.0})
        );
        let bound = Pad {
            length: Param {
                value: 1.2,
                formula: Some("3 * Printer.nozzle".into()),
            },
            depth: 4.0.into(),
        };
        let text = serde_json::to_value(&bound).unwrap();
        assert_eq!(text["length"]["formula"], "3 * Printer.nozzle");
        assert_eq!(serde_json::from_value::<Pad>(text).unwrap(), bound);
    }

    #[test]
    fn slots_read_and_write_either_shape() {
        let mut plain = json!(3.5);
        let mut bound = json!({"formula": "a.b", "value": 1.0});
        assert_eq!(read(&plain), Some((3.5, None)));
        assert_eq!(read(&bound), Some((1.0, Some("a.b"))));
        assert_eq!(read(&json!("text")), None);
        write_value(&mut plain, 4.0);
        write_value(&mut bound, 2.0);
        assert_eq!(plain, json!(4.0));
        assert_eq!(bound, json!({"formula": "a.b", "value": 2.0}));
        set(&mut bound, 7.0, None);
        assert_eq!(bound, json!(7.0));
    }
}
