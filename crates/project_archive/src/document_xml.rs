// Copyright (c) 2026 Stan Yolo
// SPDX-License-Identifier: MIT OR Apache-2.0
// See ../NOTICE for the retained permission notice.
//! Objects in `Document.xml` and typed property access.
//!
//! `<Objects>` lists names and types in creation order; `<ObjectData>` holds properties
//! by object name. Property contents retain their XML tree. Consumers choose each
//! property's meaning; this module provides accessors for its serialized value kind
//! (`<Float value>`, `<Link value>`, `<PropertyPlacement …>`).

use std::collections::BTreeMap;

use crate::coordinates::{Point3, Rotation};
use glam::{DQuat, DVec3};

use crate::archive::{ArchiveLimits, FcstdError, check_limit};

/// Bounds the parser and owned XML tree's recursive copy/drop independently of bytes.
pub const MAX_XML_DEPTH: usize = 128;
pub const MAX_XML_NODES: u32 = 1_000_000;
const MAX_XML_ALLOCATION_HINTS: u64 = 2 * MAX_XML_NODES as u64;

/// Element nesting, counted in one pass over the markup without building
/// anything: roxmltree parses nested elements by recursion, so a deep file
/// is refused here before it reaches that parser. Comments, CDATA,
/// processing instructions and quoted attribute values are stepped over, so
/// markup inside them neither nests nor hides nesting. A document type
/// declaration is refused: its internal subset could define entities that
/// expand into elements this count never sees.
fn check_xml_depth(xml: &str) -> Result<(), FcstdError> {
    let bytes = xml.as_bytes();
    let unterminated =
        |what: &str, at: usize| FcstdError::Xml(format!("unterminated {what} at byte {at}"));
    let find = |from: usize, needle: &[u8]| {
        bytes[from..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|offset| from + offset)
    };
    let mut depth = 0u64;
    let mut at = 0;
    while let Some(offset) = bytes[at..].iter().position(|&b| b == b'<') {
        let open = at + offset;
        let rest = &bytes[open..];
        if rest.starts_with(b"<!--") {
            at = find(open + 4, b"-->").ok_or_else(|| unterminated("comment", open))? + 3;
        } else if rest.starts_with(b"<![CDATA[") {
            at = find(open + 9, b"]]>").ok_or_else(|| unterminated("CDATA section", open))? + 3;
        } else if rest.starts_with(b"<?") {
            at = find(open + 2, b"?>")
                .ok_or_else(|| unterminated("processing instruction", open))?
                + 2;
        } else if rest.starts_with(b"<!") {
            return Err(FcstdError::Xml("DTD is not allowed".into()));
        } else {
            // A tag runs to the first `>` outside a quoted attribute value.
            let mut quote = None;
            let mut close = None;
            for (index, &byte) in rest.iter().enumerate().skip(1) {
                match (quote, byte) {
                    (Some(q), b) if b == q => quote = None,
                    (Some(_), _) => {}
                    (None, b'"' | b'\'') => quote = Some(byte),
                    (None, b'>') => {
                        close = Some(open + index);
                        break;
                    }
                    _ => {}
                }
            }
            let close = close.ok_or_else(|| unterminated("tag", open))?;
            if rest.starts_with(b"</") {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| FcstdError::Xml("unexpected closing element".into()))?;
            } else {
                check_limit(
                    "Document.xml element depth",
                    depth + 1,
                    MAX_XML_DEPTH as u64,
                )?;
                if bytes[close - 1] != b'/' {
                    depth += 1;
                }
            }
            at = close + 1;
        }
    }
    Ok(())
}

/// An XML element owned independently of its input text.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlElement {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlElement>,
}

impl XmlElement {
    fn from_node(node: roxmltree::Node) -> Self {
        Self {
            tag: node.tag_name().name().to_owned(),
            attrs: node
                .attributes()
                .map(|attr| (attr.name().to_owned(), attr.value().to_owned()))
                .collect(),
            children: node
                .children()
                .filter(|n| n.is_element())
                .map(Self::from_node)
                .collect(),
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn child(&self, tag: &str) -> Option<&XmlElement> {
        self.children.iter().find(|child| child.tag == tag)
    }

    pub fn children_named<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a XmlElement> + 'a {
        self.children.iter().filter(move |child| child.tag == tag)
    }
}

/// An object's property: its archive type and the children of `<Property>`.
#[derive(Debug, Clone, PartialEq)]
pub struct FcProperty {
    pub type_name: String,
    pub value: Vec<XmlElement>,
}

/// A `Base::Placement`: translation and rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub position: Point3,
    pub rotation: Rotation,
}

impl Placement {
    pub const IDENTITY: Self = Self {
        position: Point3::ZERO,
        rotation: Rotation::IDENTITY,
    };

    /// `self · local`: compose a group's child placement in its parent's frame.
    /// Children of `PartDesign::Body` store placements in the body's frame.
    pub fn compose(self, local: Self, object: &str) -> Result<Self, FcstdError> {
        let rotation = checked_rotation(object, "parent Placement.rotation", self.rotation)?;
        let local_rotation = checked_rotation(object, "local Placement.rotation", local.rotation)?;
        let composed = Self {
            position: Point3::from_array(
                (DVec3::from_array(self.position.to_array())
                    + rotation * DVec3::from_array(local.position.to_array()))
                .to_array(),
            ),
            rotation: Rotation((rotation * local_rotation).to_array()),
        };
        for (axis, value) in ["x", "y", "z"]
            .into_iter()
            .zip(composed.position.to_array())
        {
            check_finite(
                object,
                &format!("composed Placement.position.{axis}"),
                value,
            )?;
        }
        Ok(Self {
            rotation: Rotation(
                checked_rotation(object, "composed Placement.rotation", composed.rotation)?
                    .to_array(),
            ),
            ..composed
        })
    }

    /// Placement equality at the format's decimal serialization precision.
    pub fn same_as(self, other: Self) -> bool {
        (DVec3::from_array(self.position.to_array()) - DVec3::from_array(other.position.to_array()))
            .length()
            < 1e-9
            && DQuat::from_array(self.rotation.to_array())
                .dot(DQuat::from_array(other.rotation.to_array()))
                .abs()
                > 1.0 - 1e-12
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FcObject {
    pub name: String,
    pub type_name: String,
    pub properties: BTreeMap<String, FcProperty>,
}

/// An archive document with objects in `<Objects>` order.
#[derive(Debug, Clone, PartialEq)]
pub struct FcDocument {
    pub program_version: String,
    pub objects: Vec<FcObject>,
}

impl FcDocument {
    pub fn parse(xml: &str) -> Result<Self, FcstdError> {
        check_limit(
            "Document.xml bytes",
            xml.len() as u64,
            ArchiveLimits::default().xml.0,
        )?;
        // roxmltree reserves node/attribute arrays from raw '<' and '=' counts.
        // Punctuation in a large comment or CDATA can inflate those reservations
        // independently of its actual nodes, so bound the combined allocation hint.
        let hints = xml
            .bytes()
            .filter(|byte| matches!(byte, b'<' | b'='))
            .count();
        check_limit(
            "Document.xml allocation hints",
            hints as u64,
            MAX_XML_ALLOCATION_HINTS,
        )?;
        // roxmltree's node limit does not bound how deep its recursive
        // tokenizer goes, so nesting is counted before it runs.
        check_xml_depth(xml)?;
        let tree = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                nodes_limit: MAX_XML_NODES,
                ..Default::default()
            },
        )
        .map_err(|error| FcstdError::Xml(error.to_string()))?;
        let root = tree.root_element();
        if root.tag_name().name() != "Document" {
            return Err(FcstdError::Xml(format!(
                "root element is <{}>, not <Document>",
                root.tag_name().name()
            )));
        }
        let program_version = root
            .attribute("ProgramVersion")
            .unwrap_or_default()
            .to_owned();
        let section = |tag: &str| {
            root.children()
                .find(|n| n.has_tag_name(tag))
                .ok_or_else(|| FcstdError::Xml(format!("no <{tag}> section")))
        };
        let mut properties: BTreeMap<String, BTreeMap<String, FcProperty>> = BTreeMap::new();
        for data in section("ObjectData")?
            .children()
            .filter(|n| n.has_tag_name("Object"))
        {
            let name = data
                .attribute("name")
                .ok_or_else(|| FcstdError::Xml("<ObjectData> object without name".into()))?;
            let mut props = BTreeMap::new();
            if let Some(list) = data.children().find(|n| n.has_tag_name("Properties")) {
                for prop in list.children().filter(|n| n.has_tag_name("Property")) {
                    let (Some(prop_name), Some(type_name)) =
                        (prop.attribute("name"), prop.attribute("type"))
                    else {
                        return Err(FcstdError::Xml(format!(
                            "object {name}: property without name or type"
                        )));
                    };
                    props.insert(
                        prop_name.to_owned(),
                        FcProperty {
                            type_name: type_name.to_owned(),
                            value: prop
                                .children()
                                .filter(|n| n.is_element())
                                .map(XmlElement::from_node)
                                .collect(),
                        },
                    );
                }
            }
            properties.insert(name.to_owned(), props);
        }
        let mut objects = Vec::new();
        for entry in section("Objects")?
            .children()
            .filter(|n| n.has_tag_name("Object"))
        {
            let (Some(name), Some(type_name)) = (entry.attribute("name"), entry.attribute("type"))
            else {
                return Err(FcstdError::Xml(
                    "<Objects> entry without name or type".into(),
                ));
            };
            objects.push(FcObject {
                name: name.to_owned(),
                type_name: type_name.to_owned(),
                properties: properties.remove(name).unwrap_or_default(),
            });
        }
        Ok(Self {
            program_version,
            objects,
        })
    }

    pub fn object(&self, name: &str) -> Option<&FcObject> {
        self.objects.iter().find(|object| object.name == name)
    }
}

pub(crate) fn parse_f64(object: &str, what: &str, text: &str) -> Result<f64, FcstdError> {
    let value: f64 = text.trim().parse().map_err(|_| FcstdError::Format {
        object: object.to_owned(),
        reason: format!("{what}: {text:?} is not a number"),
    })?;
    check_finite(object, what, value)?;
    Ok(value)
}

pub(crate) fn check_finite(object: &str, what: &str, value: f64) -> Result<(), FcstdError> {
    if !value.is_finite() {
        return Err(FcstdError::Format {
            object: object.to_owned(),
            reason: format!("{what}: {value} is not finite"),
        });
    }
    Ok(())
}

fn checked_rotation(object: &str, what: &str, rotation: Rotation) -> Result<DQuat, FcstdError> {
    for (index, value) in rotation.to_array().into_iter().enumerate() {
        check_finite(object, &format!("{what}.Q{index}"), value)?;
    }
    let rotation = DQuat::from_array(rotation.to_array());
    let length = rotation.length();
    // The format stores a unit quaternion with 16 digits. Reject a zero or
    // overflowing norm before normalization, and accept only decimal roundoff.
    let reason = if length == 0.0 {
        Some("zero quaternion".to_owned())
    } else if !length.is_finite() {
        Some("quaternion norm is not finite".to_owned())
    } else if (length - 1.0).abs() > 1e-9 {
        Some(format!("rotation quaternion length {length} is not unit"))
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(FcstdError::Format {
            object: object.to_owned(),
            reason: format!("{what}: {reason}"),
        });
    }
    Ok(rotation.normalize())
}

impl FcObject {
    pub fn format_error(&self, reason: impl Into<String>) -> FcstdError {
        FcstdError::Format {
            object: self.name.clone(),
            reason: reason.into(),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.properties.contains_key(name)
    }

    pub fn property(&self, name: &str) -> Result<&FcProperty, FcstdError> {
        self.properties
            .get(name)
            .ok_or_else(|| self.format_error(format!("no property {name}")))
    }

    /// The first property element with the given tag.
    fn element(&self, name: &str, tag: &str) -> Result<&XmlElement, FcstdError> {
        self.property(name)?
            .value
            .iter()
            .find(|element| element.tag == tag)
            .ok_or_else(|| self.format_error(format!("property {name} has no <{tag}>")))
    }

    fn element_attr<'a>(
        &self,
        name: &str,
        element: &'a XmlElement,
        attr: &str,
    ) -> Result<&'a str, FcstdError> {
        element.attr(attr).ok_or_else(|| {
            self.format_error(format!("property {name}: <{}> has no {attr}", element.tag))
        })
    }

    pub fn attr_f64(
        &self,
        what: &str,
        element: &XmlElement,
        attr: &str,
    ) -> Result<f64, FcstdError> {
        let text = self.element_attr(what, element, attr)?;
        parse_f64(&self.name, &format!("{what}.{attr}"), text)
    }

    /// An integer attribute, such as a type code or geometry index.
    /// Fractional text such as `"1.5"` is rejected without rounding.
    pub fn attr_i64(
        &self,
        what: &str,
        element: &XmlElement,
        attr: &str,
    ) -> Result<i64, FcstdError> {
        let text = self.element_attr(what, element, attr)?;
        text.trim()
            .parse()
            .map_err(|_| self.format_error(format!("{what}.{attr}: {text:?} is not an integer")))
    }

    /// `App::PropertyFloat`, `PropertyLength` or `PropertyAngle` (degrees): `<Float value>`.
    pub fn float(&self, name: &str) -> Result<f64, FcstdError> {
        let element = self.element(name, "Float")?;
        self.attr_f64(name, element, "value")
    }

    pub fn bool(&self, name: &str) -> Result<bool, FcstdError> {
        let element = self.element(name, "Bool")?;
        match self.element_attr(name, element, "value")? {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(self.format_error(format!("property {name}: bool {other:?}"))),
        }
    }

    /// `App::PropertyEnumeration` or `PropertyInteger`: `<Integer value>`.
    pub fn integer(&self, name: &str) -> Result<i64, FcstdError> {
        let element = self.element(name, "Integer")?;
        let text = self.element_attr(name, element, "value")?;
        text.parse()
            .map_err(|_| self.format_error(format!("property {name}: integer {text:?}")))
    }

    pub fn string(&self, name: &str) -> Result<&str, FcstdError> {
        let element = self.element(name, "String")?;
        self.element_attr(name, element, "value")
    }

    /// `App::PropertyLink`. An empty link means absence, as with a body's first
    /// feature having no base, and is represented by `None`.
    pub fn link(&self, name: &str) -> Result<Option<&str>, FcstdError> {
        let element = self.element(name, "Link")?;
        let value = self.element_attr(name, element, "value")?;
        Ok((!value.is_empty()).then_some(value))
    }

    /// `App::PropertyLinkSub`: object and subelement names (empty means the whole object).
    pub fn link_sub(&self, name: &str) -> Result<Option<(&str, Vec<&str>)>, FcstdError> {
        let element = self.element(name, "LinkSub")?;
        let value = self.element_attr(name, element, "value")?;
        if value.is_empty() {
            return Ok(None);
        }
        let subs = element
            .children_named("Sub")
            .map(|sub| self.element_attr(name, sub, "value"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some((
            value,
            subs.into_iter().filter(|sub| !sub.is_empty()).collect(),
        )))
    }

    /// `App::PropertyLinkSubList`: pairs of object and subelement names.
    pub fn link_sub_list(&self, name: &str) -> Result<Vec<(&str, &str)>, FcstdError> {
        let element = self.element(name, "LinkSubList")?;
        element
            .children_named("Link")
            .map(|link| {
                Ok((
                    self.element_attr(name, link, "obj")?,
                    self.element_attr(name, link, "sub")?,
                ))
            })
            .collect()
    }

    /// `App::PropertyLinkList`: object names.
    pub fn link_list(&self, name: &str) -> Result<Vec<&str>, FcstdError> {
        let element = self.element(name, "LinkList")?;
        element
            .children_named("Link")
            .map(|link| self.element_attr(name, link, "value"))
            .collect()
    }

    /// `App::PropertyPlacement`: `Px Py Pz` and quaternion `Q0..Q3` = (x, y, z, w).
    pub fn placement(&self, name: &str) -> Result<Placement, FcstdError> {
        let element = self.element(name, "PropertyPlacement")?;
        let f = |attr| self.attr_f64(name, element, attr);
        let rotation = checked_rotation(
            &self.name,
            name,
            Rotation([f("Q0")?, f("Q1")?, f("Q2")?, f("Q3")?]),
        )?;
        Ok(Placement {
            position: Point3::new(f("Px")?, f("Py")?, f("Pz")?),
            rotation: Rotation(rotation.to_array()),
        })
    }

    /// `Part::PropertyPartShape`: the saved shape's entry name (`<Part file>`).
    pub fn shape_file(&self, name: &str) -> Result<&str, FcstdError> {
        let element = self.element(name, "Part")?;
        self.element_attr(name, element, "file")
    }

    /// `ExpressionEngine` formulas: property path and expression text.
    pub fn expressions(&self) -> Result<Vec<(String, String)>, FcstdError> {
        if !self.has("ExpressionEngine") {
            return Ok(Vec::new());
        }
        let element = self.element("ExpressionEngine", "ExpressionEngine")?;
        element
            .children_named("Expression")
            .map(|expr| {
                Ok((
                    self.element_attr("ExpressionEngine", expr, "path")?
                        .to_owned(),
                    self.element_attr("ExpressionEngine", expr, "expression")?
                        .to_owned(),
                ))
            })
            .collect()
    }

    /// Object visibility; objects without this property in older files are visible.
    pub fn visible(&self) -> Result<bool, FcstdError> {
        if self.has("Visibility") {
            self.bool("Visibility")
        } else {
            Ok(true)
        }
    }
}
