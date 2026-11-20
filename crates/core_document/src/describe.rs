//! What a feature's fields are, as a script writes them: each field's
//! name, kind and default, and for a field of several forms each form
//! with its own fields. A type derives [`Describe`] (`#[derive(Walk)]`
//! derives it too), reading its fields' walk roles for their kinds and
//! serde's attributes for their names and defaults, so the description
//! is the type's own definition. [`render`] writes it a line per field.
//!
//! A type from a crate below this one is described through serde alone
//! ([`by_serde`]): its fields' and forms' names, without kinds.

use serde::de::{self, DeserializeOwned, IntoDeserializer, Visitor};
use serde_json::Value as Json;

pub use feature_walk::Describe;
#[doc(hidden)]
pub use serde_json::Value;

/// What a value is written as.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A number of the kind named: `number`, `length`, `angle` or `count`.
    Number(&'static str),
    Bool,
    Text,
    /// A feature's or a body's id.
    Id,
    List(Box<Shape>),
    /// A list of exactly so many.
    Array(usize, Box<Shape>),
    /// Left out, it is none (`nil`).
    Optional(Box<Shape>),
    /// A table of named fields.
    Fields(Vec<Field>),
    /// One of several forms, each by its name: alone for a form holding
    /// nothing, `{Name = ...}` for one holding something.
    Variants(Vec<Variant>),
    /// Any one of these, written as itself.
    OneOf(Vec<Shape>),
    /// Written as these words say.
    Words(&'static str),
    /// Not known: a field serde names without saying what it holds.
    Unknown,
}

/// One field of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub shape: Shape,
    /// What it is when left out; `None` when it must be given.
    pub default: Option<Json>,
    /// Its fields are written beside the table's own.
    pub flatten: bool,
}

/// One form of a [`Shape::Variants`]: its name and what it holds, if
/// anything.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub shape: Option<Shape>,
}

/// A type whose written form can be described.
pub trait Describe {
    fn shape() -> Shape;
}

/// `value` as JSON, for a default.
#[doc(hidden)]
pub fn json<T: serde::Serialize>(value: &T) -> Option<Json> {
    serde_json::to_value(value).ok()
}

macro_rules! described {
    ($shape:expr => $($t:ty),*) => {$(
        impl Describe for $t {
            fn shape() -> Shape {
                $shape
            }
        }
    )*};
}
described!(Shape::Number("number") => f32, f64, u8, u16, u32, u64, usize, i32, i64);
described!(Shape::Bool => bool);
described!(Shape::Text => String);
described!(Shape::Id => uuid::Uuid, crate::FeatureId, crate::BodyId);
described!(Shape::Words("any value") => Json);

impl<T: Describe> Describe for Option<T> {
    fn shape() -> Shape {
        Shape::Optional(Box::new(T::shape()))
    }
}

impl<T: Describe> Describe for Vec<T> {
    fn shape() -> Shape {
        Shape::List(Box::new(T::shape()))
    }
}

impl<T: Describe, const N: usize> Describe for [T; N] {
    fn shape() -> Shape {
        Shape::Array(N, Box::new(T::shape()))
    }
}

impl<T: Describe> Describe for Box<T> {
    fn shape() -> Shape {
        T::shape()
    }
}

macro_rules! by_serde {
    ($($t:ty),*) => {$(
        impl Describe for $t {
            fn shape() -> Shape {
                by_serde::<$t>()
            }
        }
    )*};
}
by_serde!(
    crate::BasePlane,
    crate::DatumAttachment,
    crate::AttachmentOffset,
    kernel_api::ProjectMode,
    kernel_api::ThicknessJoin,
    kernel_api::RibSide,
    kernel_api::PrimitiveKind,
    kernel_api::Placement,
    kernel_api::FilletCorner,
    kernel_api::ChamferCorner,
    kernel_api::BoolKind,
    kernel_api::FaceSurface
);

/// `T` as serde reads it: a table's field names, a type's forms with
/// the field names of each, a list, an option, a number, a bool or text.
/// Serde says nothing of a field's own type, so those are [`Shape::Unknown`].
pub fn by_serde<T: DeserializeOwned>() -> Shape {
    let shape = trace::<T>(None);
    let Shape::Variants(variants) = shape else {
        return shape;
    };
    let variants = (0..variants.len())
        .map(|i| match trace::<T>(Some(i)) {
            Shape::Variants(mut one) if one.len() == 1 => one.remove(0),
            _ => variants[i].clone(),
        })
        .collect();
    Shape::Variants(variants)
}

/// What deserializing `T` asks for, choosing form `pick` of an enum.
fn trace<T: DeserializeOwned>(pick: Option<usize>) -> Shape {
    let mut found = None;
    let _ = T::deserialize(Probe {
        found: &mut found,
        pick,
    });
    found.unwrap_or(Shape::Unknown)
}

/// A deserializer that answers nothing and notes what it was asked for.
struct Probe<'a> {
    found: &'a mut Option<Shape>,
    /// The form of an enum to go into: `None` notes the forms' names.
    pick: Option<usize>,
}

#[derive(Debug)]
struct Traced;

impl std::fmt::Display for Traced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("traced")
    }
}

impl std::error::Error for Traced {}

impl de::Error for Traced {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Traced
    }
}

impl Probe<'_> {
    fn note<T>(self, shape: Shape) -> Result<T, Traced> {
        *self.found = Some(shape);
        Err(Traced)
    }

    /// What one value inside this one is.
    fn inner<'de, S: de::DeserializeSeed<'de>>(seed: S) -> Shape {
        let mut found = None;
        let _ = seed.deserialize(Probe {
            found: &mut found,
            pick: None,
        });
        found.unwrap_or(Shape::Unknown)
    }
}

fn names(fields: &[&str]) -> Shape {
    Shape::Fields(
        fields
            .iter()
            .map(|name| Field {
                name: name.to_string(),
                shape: Shape::Unknown,
                default: None,
                flatten: false,
            })
            .collect(),
    )
}

macro_rules! noted {
    ($shape:expr => $($method:ident),*) => {$(
        fn $method<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Traced> {
            self.note($shape)
        }
    )*};
}

impl<'de> de::Deserializer<'de> for Probe<'_> {
    type Error = Traced;

    noted!(Shape::Number("number") => deserialize_i8, deserialize_i16, deserialize_i32,
        deserialize_i64, deserialize_u8, deserialize_u16, deserialize_u32, deserialize_u64,
        deserialize_f32, deserialize_f64);
    noted!(Shape::Bool => deserialize_bool);
    noted!(Shape::Text => deserialize_char, deserialize_str, deserialize_string);
    noted!(Shape::Unknown => deserialize_any, deserialize_bytes, deserialize_byte_buf,
        deserialize_unit, deserialize_map, deserialize_identifier, deserialize_ignored_any);

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Traced> {
        let mut found = None;
        let _ = visitor.visit_some(Probe {
            found: &mut found,
            pick: None,
        });
        self.note(Shape::Optional(Box::new(found.unwrap_or(Shape::Unknown))))
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Traced> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Traced> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Traced> {
        let mut item = None;
        let _ = visitor.visit_seq(Items { item: &mut item });
        self.note(Shape::List(Box::new(item.unwrap_or(Shape::Unknown))))
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Traced> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: usize,
        visitor: V,
    ) -> Result<V::Value, Traced> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Traced> {
        self.note(names(fields))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Traced> {
        let Some(pick) = self.pick.filter(|i| *i < variants.len()) else {
            let named = variants
                .iter()
                .map(|name| Variant {
                    name: name.to_string(),
                    shape: None,
                })
                .collect();
            return self.note(Shape::Variants(named));
        };
        let mut shape = None;
        let _ = visitor.visit_enum(Form {
            pick,
            shape: &mut shape,
        });
        self.note(Shape::Variants(vec![Variant {
            name: variants[pick].to_string(),
            shape,
        }]))
    }
}

/// A list whose first item is traced.
struct Items<'a> {
    item: &'a mut Option<Shape>,
}

impl<'de> de::SeqAccess<'de> for Items<'_> {
    type Error = Traced;

    fn next_element_seed<S: de::DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Traced> {
        *self.item = Some(Probe::inner(seed));
        Err(Traced)
    }
}

/// The form of an enum being traced.
struct Form<'a> {
    pick: usize,
    shape: &'a mut Option<Shape>,
}

impl<'de, 'a> de::EnumAccess<'de> for Form<'a> {
    type Error = Traced;
    type Variant = Self;

    fn variant_seed<S: de::DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Self), Traced> {
        let index: de::value::U32Deserializer<Traced> = (self.pick as u32).into_deserializer();
        Ok((seed.deserialize(index)?, self))
    }
}

impl<'de> de::VariantAccess<'de> for Form<'_> {
    type Error = Traced;

    fn unit_variant(self) -> Result<(), Traced> {
        Ok(())
    }

    fn newtype_variant_seed<S: de::DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<S::Value, Traced> {
        *self.shape = Some(Probe::inner(seed));
        Err(Traced)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value, Traced> {
        *self.shape = Some(Shape::List(Box::new(Shape::Unknown)));
        Err(Traced)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Traced> {
        *self.shape = Some(names(fields));
        Err(Traced)
    }
}

/// How a description starts: how to read its lines.
pub const READING: &str = "Fields, one a line, `name: kind = what it is when left out` \
(`(required)`: nothing is, so give it). A field of kind `form` is one of the forms listed \
under it: a form with no fields by its name alone (`\"Name\"`), any other as `{Name = {its \
fields}}`, or `{Name = value}` where the line says `Name = kind`.";

/// `fields` a line each, as [`READING`] says, the values of `given` (a
/// table of the same fields) as their defaults where it has them; those
/// named in `from_call` are said to come from what the call gives. A form
/// or table met a second time is named after the field it was first
/// described under.
pub fn render(fields: &[Field], given: Option<&Json>, from_call: &[String]) -> String {
    let mut out = Writer {
        from_call,
        ..Writer::default()
    };
    out.fields(fields, given, 0);
    out.lines.join("\n")
}

#[derive(Default)]
struct Writer<'a> {
    lines: Vec<String>,
    /// Top-level fields whose value comes from the call.
    from_call: &'a [String],
    /// Forms and tables described in full, by their names, and the field
    /// each was described under.
    seen: Vec<(String, String)>,
}

impl Writer<'_> {
    fn fields(&mut self, fields: &[Field], given: Option<&Json>, depth: usize) {
        for field in fields {
            if field.flatten {
                if let Shape::Fields(inner) = &field.shape {
                    self.fields(inner, given, depth);
                }
                continue;
            }
            let called = depth == 0 && self.from_call.contains(&field.name);
            let value = given
                .and_then(|g| g.get(&field.name))
                .or(field.default.as_ref());
            let shown = value.filter(|_| !called);
            let again = key(&field.shape).and_then(|k| {
                let first = self
                    .seen
                    .iter()
                    .find(|(seen, _)| *seen == k)
                    .map(|(_, first)| first.clone());
                if first.is_none() {
                    self.seen.push((k, field.name.clone()));
                }
                first
            });
            let mut line = format!("{}{}: ", "  ".repeat(depth), field.name);
            line.push_str(&match (&again, &field.shape) {
                (Some(first), Shape::List(_) | Shape::Array(..)) => {
                    format!("list, each as `{first}`")
                }
                (Some(first), _) => format!("as `{first}`"),
                (None, shape) => kind_of(shape, shown),
            });
            match value {
                Some(_) if called => line.push_str(" = what the call gives"),
                // A table's values are written in it, or under it.
                Some(value) if value.is_object() && table(&field.shape) && again.is_none() => {}
                Some(value) => line.push_str(&format!(" = {}", lua(value))),
                None if !matches!(field.shape, Shape::Unknown) => line.push_str(" (required)"),
                None => {}
            }
            self.lines.push(line);
            if again.is_none() {
                self.nested(&field.shape, shown, depth + 1);
            }
        }
    }

    /// The lines under a field: a table's fields, or its forms.
    fn nested(&mut self, shape: &Shape, value: Option<&Json>, depth: usize) {
        match shape {
            Shape::Optional(inner) => self.nested(inner, value, depth),
            Shape::List(inner) | Shape::Array(_, inner) => self.nested(inner, None, depth),
            Shape::Fields(fields) if !inline(fields) => self.fields(fields, value, depth),
            Shape::Variants(variants) if !unit_only(variants) => {
                let pad = "  ".repeat(depth);
                for variant in variants {
                    let held = value
                        .and_then(Json::as_object)
                        .filter(|m| m.len() == 1)
                        .and_then(|m| m.get(&variant.name));
                    match &variant.shape {
                        None => self.lines.push(format!("{pad}{}", variant.name)),
                        Some(Shape::Fields(fields)) if inline(fields) => self.lines.push(format!(
                            "{pad}{} {}",
                            variant.name,
                            braces(fields, held)
                        )),
                        Some(Shape::Fields(fields)) => {
                            self.lines.push(format!("{pad}{}:", variant.name));
                            self.fields(fields, held, depth + 1);
                        }
                        Some(other) => {
                            let kind = kind_of(other, held);
                            self.lines.push(format!("{pad}{} = {kind}", variant.name));
                            self.nested(other, held, depth + 1);
                        }
                    }
                }
            }
            Shape::OneOf(shapes) => {
                for shape in shapes {
                    self.nested(shape, None, depth);
                }
            }
            _ => {}
        }
    }
}

/// What names a table or a set of forms worth describing once: its
/// fields' or forms' names.
fn key(shape: &Shape) -> Option<String> {
    let names = |names: Vec<&str>| Some(names.join(","));
    match shape {
        Shape::Optional(inner) | Shape::List(inner) | Shape::Array(_, inner) => key(inner),
        Shape::Fields(fields) if fields.len() >= 3 && !unknown(fields) => {
            names(fields.iter().map(|f| f.name.as_str()).collect())
        }
        Shape::Variants(variants) if !unit_only(variants) => {
            names(variants.iter().map(|v| v.name.as_str()).collect())
        }
        _ => None,
    }
}

/// Whether a value of `shape` is a table whose values its description
/// writes out.
fn table(shape: &Shape) -> bool {
    match shape {
        Shape::Optional(inner) => table(inner),
        Shape::Fields(_) => true,
        _ => false,
    }
}

fn unit_only(variants: &[Variant]) -> bool {
    variants.iter().all(|v| v.shape.is_none())
}

/// Fields serde names without saying what they hold.
fn unknown(fields: &[Field]) -> bool {
    fields.iter().all(|f| f.shape == Shape::Unknown)
}

/// A shape that needs no lines under it.
fn leaf(shape: &Shape) -> bool {
    match shape {
        Shape::Optional(inner) | Shape::List(inner) | Shape::Array(_, inner) => leaf(inner),
        Shape::Fields(fields) => inline(fields),
        Shape::Variants(variants) => unit_only(variants),
        Shape::OneOf(shapes) => shapes.iter().all(leaf),
        _ => true,
    }
}

/// A table short and plain enough to write on its field's line.
fn inline(fields: &[Field]) -> bool {
    unknown(fields) || (fields.len() <= 4 && fields.iter().all(|f| !f.flatten && leaf(&f.shape)))
}

/// A short table on one line: `{name: kind = default, ...}`, the values
/// `given` holds as the defaults.
fn braces(fields: &[Field], given: Option<&Json>) -> String {
    let given = given.filter(|g| g.is_object());
    let inner: Vec<String> = fields
        .iter()
        .map(|f| {
            let value = given.and_then(|g| g.get(&f.name)).or(f.default.as_ref());
            match (&f.shape, value) {
                (Shape::Unknown, None) => f.name.clone(),
                (Shape::Unknown, Some(value)) => format!("{} = {}", f.name, lua(value)),
                (shape, Some(value)) => format!("{}: {} = {}", f.name, kind(shape), lua(value)),
                (shape, None) => format!("{}: {} (required)", f.name, kind(shape)),
            }
        })
        .collect();
    format!("{{{}}}", inner.join(", "))
}

/// The kind of a value of `shape` holding `value`: a short table's with
/// the values it holds.
fn kind_of(shape: &Shape, value: Option<&Json>) -> String {
    match shape {
        Shape::Optional(inner) => kind_of(inner, value),
        Shape::Fields(fields) if inline(fields) => braces(fields, value),
        other => kind(other),
    }
}

/// The kind of a value, as its field's line says it.
fn kind(shape: &Shape) -> String {
    match shape {
        Shape::Number(kind) => kind.to_string(),
        Shape::Bool => "bool".into(),
        Shape::Text => "text".into(),
        Shape::Id => "id".into(),
        Shape::Words(words) => words.to_string(),
        Shape::Unknown => "value".into(),
        Shape::Optional(inner) => kind(inner),
        Shape::List(inner) => format!("list of {}", kind(inner)),
        Shape::Array(n, inner) => format!("list of {n} {}", kind(inner)),
        Shape::Fields(fields) if inline(fields) => braces(fields, None),
        Shape::Fields(_) => "table".into(),
        Shape::Variants(variants) if unit_only(variants) => variants
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>()
            .join(" | "),
        Shape::Variants(_) => "form".into(),
        Shape::OneOf(shapes) => shapes.iter().map(kind).collect::<Vec<_>>().join(" or "),
    }
}
/// The longest default written out in full.
const LONGEST: usize = 90;

/// `value` as Lua writes it; an id as `<id>`, since it names something
/// of the document the description was made from.
fn lua(value: &Json) -> String {
    let text = match value {
        Json::Null => "nil".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => match n.as_f64() {
            // A single's digits, as the feature keeps it.
            Some(f) if n.is_f64() && f64::from(f as f32) == f => (f as f32).to_string(),
            _ => n.to_string(),
        },
        Json::String(s) if uuid::Uuid::parse_str(s).is_ok() => "<id>".to_string(),
        Json::String(s) => format!("{s:?}"),
        Json::Array(items) => format!(
            "{{{}}}",
            items.iter().map(lua).collect::<Vec<_>>().join(", ")
        ),
        Json::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| format!("{k} = {}", lua(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    if text.chars().count() > LONGEST {
        let cut: String = text.chars().take(LONGEST).collect();
        format!("{cut}…")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serde alone names a type's forms and each form's fields, and a
    /// table's fields; it reaches into options and lists.
    #[test]
    fn serde_names_forms_and_fields() {
        let Shape::Variants(forms) = by_serde::<kernel_api::PrimitiveKind>() else {
            panic!("a primitive is one of several forms");
        };
        let cylinder = forms.iter().find(|v| v.name == "Cylinder").unwrap();
        assert!(
            matches!(&cylinder.shape, Some(Shape::Fields(f)) if f.iter().any(|f| f.name == "radius"))
        );
        assert_eq!(
            by_serde::<Option<Vec<f32>>>(),
            Shape::Optional(Box::new(Shape::List(Box::new(Shape::Number("number")))))
        );
    }
}
