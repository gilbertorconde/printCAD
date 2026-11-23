//! Names for the faces and edges of a built solid that survive a rebuild.
//!
//! A face is named by how it came to be: the feature that made it and the
//! sketch element it was swept from, or the feature's start or end. A
//! face a later feature splits, trims or merges keeps the names of the
//! faces it came from; an edge is named by the two faces it runs between.
//! References keep these names beside their points, so a rebuild finds a
//! face again after dimensions change or features go in earlier, where a
//! point alone would miss it.
//!
//! A name is a 64-bit hash of its parts, the same on every machine and in
//! every version: it is saved in documents.

/// A face's name; zero is no name.
pub type TopoName = u64;

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0100_0000_01b3;

fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The name of `bytes`: never zero.
pub fn name_of(bytes: &[u8]) -> TopoName {
    match fnv(OFFSET, bytes) {
        0 => 1,
        name => name,
    }
}

/// The name of a thing with a UUID's bytes (a feature, a sketch element).
pub fn name_of_id(id: &[u8; 16]) -> TopoName {
    name_of(id)
}

/// `parent`'s child called `part`: a feature's face swept from one of its
/// sketch's elements, or its start.
pub fn child(parent: TopoName, part: &[u8]) -> TopoName {
    match fnv(fnv(OFFSET, &parent.to_le_bytes()), part) {
        0 => 1,
        name => name,
    }
}

/// Reads a name written as a number or as a string of its digits: a
/// script's numbers are doubles, which cannot hold every name, so names
/// reach scripts as strings and come back as them. Anything else (a
/// fraction, a negative number, a double past 2^53 that may have lost its
/// last digits) is refused, naming the field `name`.
pub fn name_from_number_or_text<'de, D>(deserializer: D) -> Result<TopoName, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(NameVisitor { field: "name" })
}

/// Reads a pair of names, each written as a number or as a string of its
/// digits, as [`name_from_number_or_text`] reads one: an edge's two faces,
/// its refusal naming the field `faces`.
pub fn names_from_numbers_or_text<'de, D>(deserializer: D) -> Result<[TopoName; 2], D::Error>
where
    D: serde::Deserializer<'de>,
{
    name_pair(deserializer, "faces")
}

/// Reads a pair of names as [`names_from_numbers_or_text`] does, its
/// refusal naming `field`.
pub fn name_pair<'de, D>(deserializer: D, field: &'static str) -> Result<[TopoName; 2], D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Pair(&'static str);
    impl<'de> serde::de::Visitor<'de> for Pair {
        type Value = [TopoName; 2];
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "`{}` as two face names", self.0)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut next = |i: usize| {
                seq.next_element_seed(NameVisitor { field: self.0 })?
                    .ok_or_else(|| serde::de::Error::invalid_length(i, &self))
            };
            let pair = [next(0)?, next(1)?];
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom(format!(
                    "`{}` holds two face names, no more",
                    self.0
                )));
            }
            Ok(pair)
        }
    }
    deserializer.deserialize_seq(Pair(field))
}

/// One name of the field `field`, from a number or a string of its digits.
#[derive(Clone, Copy)]
struct NameVisitor {
    field: &'static str,
}

impl NameVisitor {
    fn refuse<E: serde::de::Error>(self, what: impl std::fmt::Display) -> E {
        E::custom(format!(
            "`{}` takes face names, each a whole number from 0 to 2^64 - 1, best \
             written as a string of its digits as feature reads give them; it was given {what}",
            self.field
        ))
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for NameVisitor {
    type Value = TopoName;
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<TopoName, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for NameVisitor {
    type Value = TopoName;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "a face name in `{}`", self.field)
    }

    fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<TopoName, E> {
        Ok(n)
    }

    fn visit_i64<E: serde::de::Error>(self, n: i64) -> Result<TopoName, E> {
        u64::try_from(n).map_err(|_| self.refuse(format_args!("the negative number {n}")))
    }

    fn visit_f64<E: serde::de::Error>(self, n: f64) -> Result<TopoName, E> {
        // A double holds every whole number up to 2^53 exactly; past it, a
        // name may have been rounded on the way.
        const EXACT: f64 = 9_007_199_254_740_992.0;
        if n.fract() == 0.0 && (0.0..=EXACT).contains(&n) {
            Ok(n as u64)
        } else if n.fract() == 0.0 && n > EXACT {
            Err(self.refuse(format_args!(
                "the floating-point number {n}, which may have lost digits past 2^53"
            )))
        } else {
            Err(self.refuse(format_args!("the number {n}")))
        }
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<TopoName, E> {
        text.trim()
            .parse()
            .map_err(|_| self.refuse(format_args!("the string {text:?}")))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<TopoName, E> {
        Err(self.refuse("nothing"))
    }

    fn visit_bool<E: serde::de::Error>(self, b: bool) -> Result<TopoName, E> {
        Err(self.refuse(format_args!("{b}")))
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, _: A) -> Result<TopoName, A::Error> {
        Err(self.refuse("a list"))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, _: A) -> Result<TopoName, A::Error> {
        Err(self.refuse("a table"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_fixed_and_never_zero() {
        // Saved in documents: the same bytes always make the same name.
        assert_eq!(name_of(b"pad"), 0x77ca60195676adaa);
        assert_ne!(name_of(b"pad"), name_of(b"pocket"));
        assert_ne!(
            child(name_of(b"pad"), b"start"),
            child(name_of(b"pad"), b"end")
        );
        assert_ne!(name_of(&[]), 0);
    }

    /// A script hands names back as the strings it was given them as.
    #[test]
    fn a_pair_of_names_reads_from_numbers_or_strings() {
        #[derive(serde::Deserialize)]
        struct Pick {
            #[serde(deserialize_with = "names_from_numbers_or_text")]
            faces: [TopoName; 2],
        }
        let read = |text: &str| serde_json::from_str::<Pick>(text).map(|p| p.faces);
        assert_eq!(
            read(r#"{"faces": ["18446744073709551615", 7]}"#).unwrap(),
            [u64::MAX, 7]
        );
        assert!(read(r#"{"faces": ["x", 7]}"#).is_err());
        assert!(read(r#"{"faces": [1, 2, 3]}"#).is_err());
    }

    /// A value that is not a name is refused saying which field took it
    /// and what it was given: a double past 2^53 may have lost digits, so
    /// it is refused too, never rounded into a name.
    #[test]
    fn a_name_that_is_no_whole_number_is_refused_by_field() {
        #[derive(Debug, serde::Deserialize)]
        struct Pick {
            #[serde(default, deserialize_with = "name_from_number_or_text")]
            #[allow(dead_code)]
            name: TopoName,
        }
        let said = |text: &str| serde_json::from_str::<Pick>(text).unwrap_err().to_string();
        let float = said(r#"{"name": 1.1208812177732762e19}"#);
        assert!(
            float.contains("`name`") && float.contains("floating-point"),
            "{float}"
        );
        let negative = said(r#"{"name": -3}"#);
        assert!(
            negative.contains("`name`") && negative.contains("negative"),
            "{negative}"
        );
        let read = |text: &str| serde_json::from_str::<Pick>(text).map(|p| p.name).ok();
        assert_eq!(
            read(r#"{"name": 7.0}"#),
            Some(7),
            "a whole double under 2^53"
        );
        assert_eq!(
            read(r#"{"name": 9223372036854775807}"#),
            Some(i64::MAX as u64)
        );
    }
}
