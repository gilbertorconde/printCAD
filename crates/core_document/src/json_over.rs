//! A value serialized over a JSON [`Value`] already standing, keeping the
//! nodes whose shape agrees: an object's entries, an array's items, a
//! string's buffer. A large feature written again after a small edit
//! (a sketch of thousands of elements) costs a walk of it, not a node
//! allocated and the old one freed for every number, key and id.
//!
//! The result is exactly what `serde_json::to_value` makes of the same
//! value; anything this writer does not take (a map key that is neither
//! a string, an integer, a character nor a boolean) is made by
//! `to_value` instead.

use serde::Serialize;
use serde::ser::{self, Impossible};
use serde_json::{Error, Map, Value};

/// `value` serialized over `target`, which then holds exactly what
/// `serde_json::to_value(value)` answers.
pub fn write_over<T: Serialize + ?Sized>(target: &mut Value, value: &T) -> Result<(), Error> {
    if value.serialize(Over(target)).is_err() {
        *target = serde_json::to_value(value)?;
    }
    Ok(())
}

/// Writes one value over the node it holds.
struct Over<'a>(&'a mut Value);

fn plain() -> serde_json::value::Serializer {
    serde_json::value::Serializer
}

fn unsupported() -> Error {
    ser::Error::custom("not written over")
}

/// `target` as a string holding `text`, its buffer kept.
fn put_str(target: &mut Value, text: &str) {
    match target {
        Value::String(s) => {
            if s != text {
                s.clear();
                s.push_str(text);
            }
        }
        other => *other = Value::String(text.to_owned()),
    }
}

/// `target` as an array, its items kept.
fn array(target: &mut Value) -> &mut Vec<Value> {
    if !target.is_array() {
        *target = Value::Array(Vec::new());
    }
    match target {
        Value::Array(items) => items,
        _ => unreachable!("made an array above"),
    }
}

/// `target` as an object, its entries kept.
fn object(target: &mut Value) -> &mut Map<String, Value> {
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    match target {
        Value::Object(map) => map,
        _ => unreachable!("made an object above"),
    }
}

/// The node under an enum variant's one key, `{variant: ...}`, kept when
/// `target` already is that variant.
fn variant<'a>(target: &'a mut Value, name: &'static str) -> &'a mut Value {
    let map = object(target);
    if map.len() != 1 || !map.contains_key(name) {
        map.clear();
        map.insert(name.to_owned(), Value::Null);
    }
    map.get_mut(name).expect("the variant's key was put above")
}

/// The node under `key` in `map`, made empty when missing.
fn entry<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Value {
    if !map.contains_key(key) {
        map.insert(key.to_owned(), Value::Null);
    }
    map.get_mut(key).expect("the key was put above")
}

impl<'a> ser::Serializer for Over<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Items<'a>;
    type SerializeTuple = Items<'a>;
    type SerializeTupleStruct = Items<'a>;
    type SerializeTupleVariant = Items<'a>;
    type SerializeMap = Entries<'a>;
    type SerializeStruct = Fields<'a>;
    type SerializeStructVariant = Fields<'a>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        *self.0 = Value::Bool(v);
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_i8(plain(), v)?;
        Ok(())
    }
    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_i16(plain(), v)?;
        Ok(())
    }
    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_i32(plain(), v)?;
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_i64(plain(), v)?;
        Ok(())
    }
    fn serialize_i128(self, v: i128) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_i128(plain(), v)?;
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_u8(plain(), v)?;
        Ok(())
    }
    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_u16(plain(), v)?;
        Ok(())
    }
    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_u32(plain(), v)?;
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_u64(plain(), v)?;
        Ok(())
    }
    fn serialize_u128(self, v: u128) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_u128(plain(), v)?;
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_f32(plain(), v)?;
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_f64(plain(), v)?;
        Ok(())
    }
    fn serialize_char(self, v: char) -> Result<(), Error> {
        put_str(self.0, v.encode_utf8(&mut [0; 4]));
        Ok(())
    }
    fn serialize_str(self, v: &str) -> Result<(), Error> {
        put_str(self.0, v);
        Ok(())
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        *self.0 = ser::Serializer::serialize_bytes(plain(), v)?;
        Ok(())
    }
    fn serialize_none(self) -> Result<(), Error> {
        *self.0 = Value::Null;
        Ok(())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        *self.0 = Value::Null;
        Ok(())
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Error> {
        *self.0 = Value::Null;
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        put_str(self.0, variant);
        Ok(())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        name: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(Over(variant(self.0, name)))
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Items<'a>, Error> {
        Ok(Items {
            items: array(self.0),
            at: 0,
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<Items<'a>, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<Items<'a>, Error> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        name: &'static str,
        len: usize,
    ) -> Result<Items<'a>, Error> {
        Over(variant(self.0, name)).serialize_seq(Some(len))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Entries<'a>, Error> {
        let map = object(self.0);
        let old = std::mem::take(map);
        Ok(Entries {
            map,
            old,
            key: None,
        })
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Fields<'a>, Error> {
        Ok(Fields {
            map: object(self.0),
            written: Written::default(),
        })
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        name: &'static str,
        len: usize,
    ) -> Result<Fields<'a>, Error> {
        Over(variant(self.0, name)).serialize_struct(name, len)
    }
}

/// An array's items written over in order, the ones left over dropped.
pub struct Items<'a> {
    items: &'a mut Vec<Value>,
    at: usize,
}

impl Items<'_> {
    fn item<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        if self.at == self.items.len() {
            self.items.push(Value::Null);
        }
        let slot = &mut self.items[self.at];
        self.at += 1;
        value.serialize(Over(slot))
    }

    fn finish(self) -> Result<(), Error> {
        self.items.truncate(self.at);
        Ok(())
    }
}

impl ser::SerializeSeq for Items<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeTuple for Items<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Items<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeTupleVariant for Items<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

/// A map's entries, each written over the old entry of its key: the map
/// is made anew from them, so keys it no longer has go.
pub struct Entries<'a> {
    map: &'a mut Map<String, Value>,
    old: Map<String, Value>,
    key: Option<String>,
}

impl ser::SerializeMap for Entries<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        self.key = Some(key.serialize(Key)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        let key = self.key.take().ok_or_else(unsupported)?;
        let mut slot = self.old.remove(&key).unwrap_or(Value::Null);
        value.serialize(Over(&mut slot))?;
        self.map.insert(key, slot);
        Ok(())
    }
    fn end(self) -> Result<(), Error> {
        Ok(())
    }
}

/// The field names a struct wrote, up to [`Written::MOST`] of them.
#[derive(Default)]
struct Written {
    names: [&'static str; Written::MOST],
    count: usize,
}

impl Written {
    const MOST: usize = 32;
}

/// A struct's fields, each written over the entry of its name; entries
/// of names it did not write go.
pub struct Fields<'a> {
    map: &'a mut Map<String, Value>,
    written: Written,
}

impl Fields<'_> {
    fn field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        let written = &mut self.written;
        if let Some(name) = written.names.get_mut(written.count) {
            *name = key;
        }
        written.count += 1;
        value.serialize(Over(entry(self.map, key)))
    }

    fn finish(self) -> Result<(), Error> {
        let Written { names, count } = self.written;
        if self.map.len() != count {
            if count > Written::MOST {
                return Err(unsupported());
            }
            self.map
                .retain(|key, _| names[..count].contains(&key.as_str()));
        }
        Ok(())
    }
}

impl ser::SerializeStruct for Fields<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.field(key, value)
    }
    fn skip_field(&mut self, key: &'static str) -> Result<(), Error> {
        self.map.remove(key);
        Ok(())
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeStructVariant for Fields<'_> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.field(key, value)
    }
    fn skip_field(&mut self, key: &'static str) -> Result<(), Error> {
        self.map.remove(key);
        Ok(())
    }
    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

/// A map key as `serde_json` writes it: a string as it is, an integer,
/// a character or a boolean as its text. Any other key is left to
/// `to_value`.
struct Key;

impl ser::Serializer for Key {
    type Ok = String;
    type Error = Error;
    type SerializeSeq = Impossible<String, Error>;
    type SerializeTuple = Impossible<String, Error>;
    type SerializeTupleStruct = Impossible<String, Error>;
    type SerializeTupleVariant = Impossible<String, Error>;
    type SerializeMap = Impossible<String, Error>;
    type SerializeStruct = Impossible<String, Error>;
    type SerializeStructVariant = Impossible<String, Error>;

    fn serialize_bool(self, v: bool) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i8(self, v: i8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i128(self, v: i128) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u128(self, v: u128) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_f32(self, _v: f32) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_f64(self, _v: f64) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_char(self, v: char) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_str(self, v: &str) -> Result<String, Error> {
        Ok(v.to_owned())
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_none(self) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_unit(self) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<String, Error> {
        Ok(variant.to_owned())
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<String, Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<String, Error> {
        Err(unsupported())
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(unsupported())
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        Err(unsupported())
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(unsupported())
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(unsupported())
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(unsupported())
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Error> {
        Err(unsupported())
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(unsupported())
    }
}
