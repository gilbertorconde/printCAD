//! What a feature's data holds for the rest of the document: the numbers
//! formulas can set (each a [`Parameter`] at its JSON pointer) and the
//! features it reads, which it is rebuilt after. A feature type derives
//! [`Walk`] (`#[derive(Walk)]`, from the `feature_walk` crate), naming
//! each field's role where it is declared, so the two lists come from one
//! definition and a field whose role is not stated does not compile.
//!
//! Roles, as field attributes:
//!
//! - none: the field's type walks itself (a nested type deriving `Walk`,
//!   a [`FeatureId`], an `Option`, `Box` or `Vec` of one);
//! - `#[walk(length("name", "Label"))]`, `angle`, `number`, `count`: a
//!   number formulas can set, while it holds one (an `Option` that is
//!   `None` is not listed);
//! - `#[walk(lengths("name", "Label"))]`, `angles`, `counts`: a list of
//!   numbers, each `name1`, `name2`, … ("Label 1", …);
//! - `#[walk(axes("name_", "Label ", number))]`: a `[f32; 3]` as
//!   `name_x`, `name_y`, `name_z` ("Label X", …) of the kind given;
//! - `#[walk(steps("step", "Step"))]`: a list whose items' numbers are
//!   named after their place (`step1_length`, "Step 1 length");
//! - `#[walk(with = path)]`: a function `fn(&Field, &mut Walker)`;
//! - `#[walk(setting)]`: nothing formulas set and no feature read.
//!
//! The derive reads serde's own attributes, so the pointers are where the
//! JSON holds each number: a variant's name around its fields unless the
//! enum is `untagged`, a `flatten`ed field's fields in its place, and
//! `rename`s.

use crate::expr::Dim;
use crate::{FeatureId, Parameter};

pub use feature_walk::Walk;

/// A type whose data a [`Walker`] can go through.
pub trait Walk {
    fn walk(&self, walker: &mut Walker);

    /// Every name the type gives a number, whichever variant holds it,
    /// for checks against what the interface asks for by name.
    fn names(_out: &mut Vec<&'static str>)
    where
        Self: Sized,
    {
    }
}

/// What a walk found, and where in the data it is.
#[derive(Debug, Default)]
pub struct Walker {
    path: Vec<String>,
    /// What a list item's numbers are named and labelled after.
    item: Option<(String, String)>,
    /// The features read, each once, in the order met.
    pub references: Vec<FeatureId>,
    pub parameters: Vec<Parameter>,
}

/// A number formulas can set; `None` while the field holds none.
pub trait Number {
    fn number(&self) -> Option<f64>;
}

macro_rules! numbers {
    ($($t:ty),*) => {$(
        impl Number for $t {
            fn number(&self) -> Option<f64> {
                Some(*self as f64)
            }
        }
    )*};
}
numbers!(f32, f64, u8, u16, u32, u64, usize, i32, i64);

impl<T: Number> Number for Option<T> {
    fn number(&self) -> Option<f64> {
        self.as_ref().and_then(Number::number)
    }
}

impl Walker {
    /// Walk `value` from its top.
    pub fn of<T: Walk + ?Sized>(value: &T) -> Self {
        let mut walker = Self::default();
        value.walk(&mut walker);
        walker
    }

    /// Walk what `f` walks one step further in, at `segment`.
    pub fn at(&mut self, segment: &str, f: impl FnOnce(&mut Self)) {
        self.path.push(segment.to_string());
        f(self);
        self.path.pop();
    }

    /// A feature the data reads.
    pub fn reference(&mut self, id: FeatureId) {
        if !self.references.contains(&id) {
            self.references.push(id);
        }
    }

    fn pointer(&self) -> String {
        self.path.iter().map(|s| format!("/{s}")).collect()
    }

    /// A number at the walk's place, named `name` and labelled `label`
    /// (a `dim` of `None` is a count), when the field holds one.
    pub fn number(&mut self, value: &impl Number, name: &str, label: &str, dim: Option<Dim>) {
        if value.number().is_none() {
            return;
        }
        let (name, label) = match &self.item {
            Some((n, l)) => (
                format!("{n}_{name}"),
                format!("{l} {}", label.to_lowercase()),
            ),
            None => (name.to_string(), label.to_string()),
        };
        let pointer = self.pointer();
        self.parameters.push(match dim {
            Some(dim) => Parameter::new(&name, &label, dim, pointer),
            None => Parameter::count(&name, &label, pointer),
        });
    }

    /// A list of numbers at the walk's place: `name1`, `name2`, …
    pub fn numbers<N: Number>(&mut self, values: &[N], name: &str, label: &str, dim: Option<Dim>) {
        for (i, value) in values.iter().enumerate() {
            self.at(&i.to_string(), |w| {
                w.number(
                    value,
                    &format!("{name}{}", i + 1),
                    &format!("{label} {}", i + 1),
                    dim,
                );
            });
        }
    }

    /// Three numbers along X, Y and Z: `namex`, … labelled `labelX`, …
    pub fn axes<N: Number>(&mut self, values: &[N; 3], name: &str, label: &str, dim: Option<Dim>) {
        for (i, axis) in ["x", "y", "z"].into_iter().enumerate() {
            self.at(&i.to_string(), |w| {
                w.number(
                    &values[i],
                    &format!("{name}{axis}"),
                    &format!("{label}{}", axis.to_uppercase()),
                    dim,
                );
            });
        }
    }

    /// A list whose items' numbers take their place's name: the first
    /// item's `length` as `{name}1_length`, labelled "{Label} 1 length".
    pub fn steps<T: Walk>(&mut self, items: &[T], name: &str, label: &str) {
        for (i, item) in items.iter().enumerate() {
            let outer = self
                .item
                .replace((format!("{name}{}", i + 1), format!("{label} {}", i + 1)));
            self.at(&i.to_string(), |w| item.walk(w));
            self.item = outer;
        }
    }
}

impl Walk for FeatureId {
    fn walk(&self, walker: &mut Walker) {
        walker.reference(*self);
    }
}

impl<T: Walk> Walk for Option<T> {
    fn walk(&self, walker: &mut Walker) {
        if let Some(value) = self {
            value.walk(walker);
        }
    }

    fn names(out: &mut Vec<&'static str>) {
        T::names(out);
    }
}

impl<T: Walk> Walk for Box<T> {
    fn walk(&self, walker: &mut Walker) {
        (**self).walk(walker);
    }

    fn names(out: &mut Vec<&'static str>) {
        T::names(out);
    }
}

/// A list's items: the features they read. Their numbers are not named
/// here; a list whose items carry numbers is walked as `steps`.
impl<T: Walk> Walk for Vec<T> {
    fn walk(&self, walker: &mut Walker) {
        for (i, item) in self.iter().enumerate() {
            let before = walker.parameters.len();
            walker.at(&i.to_string(), |w| item.walk(w));
            walker.parameters.truncate(before);
        }
    }
}

impl Walk for crate::DatumAttachment {
    fn walk(&self, walker: &mut Walker) {
        for datum in self.datums() {
            walker.reference(datum);
        }
    }
}
