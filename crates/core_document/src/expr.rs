//! Formulas: what a variable or a numeric field holds when it is more
//! than a number.
//!
//! A formula is arithmetic over quantities: `3 * Printer.nozzle + 0.2 mm`,
//! `if(Bracket.width > 40, 3, 2) * Printer.layer`, `atan2(Sizes.rise,
//! Sizes.run)`. Every value carries its dimension, a power of length and a
//! power of angle, so a length field refuses an angle and `2 mm * 3 mm` is
//! an area. Lengths are held in millimetres and angles in degrees, as the
//! document holds them.
//!
//! A number written without a unit is bare. Beside a quantity in a sum, a
//! difference, a comparison or a choice it takes that quantity's unit in
//! the field's terms (the document's length unit, or degrees), and a
//! formula that comes out bare takes the unit of the field it fills, so
//! `Plate.width + 2` and `12` mean what they would in that field.
//!
//! Every reference is `object.property`, a name with anything but letters,
//! digits and `_` in it written in backticks: `` `Pad 2`.length ``. The
//! text is kept as written; [`references`] lists what a formula reads and
//! [`rename_object`] / [`rename_property`] rewrite only those names.

use std::fmt;
use std::ops::Range;

use crate::units::Unit;

/// A kind of quantity: its power of length and its power of angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Dim {
    pub length: i8,
    pub angle: i8,
}

impl Dim {
    pub const NUMBER: Dim = Dim {
        length: 0,
        angle: 0,
    };
    pub const LENGTH: Dim = Dim {
        length: 1,
        angle: 0,
    };
    pub const AREA: Dim = Dim {
        length: 2,
        angle: 0,
    };
    pub const VOLUME: Dim = Dim {
        length: 3,
        angle: 0,
    };
    pub const ANGLE: Dim = Dim {
        length: 0,
        angle: 1,
    };

    fn times(self, other: Dim) -> Dim {
        Dim {
            length: self.length + other.length,
            angle: self.angle + other.angle,
        }
    }

    fn over(self, other: Dim) -> Dim {
        Dim {
            length: self.length - other.length,
            angle: self.angle - other.angle,
        }
    }

    fn power(self, n: i8) -> Dim {
        Dim {
            length: self.length * n,
            angle: self.angle * n,
        }
    }

    /// What it is, for messages: "a length", "an angle", "mm²·deg".
    pub fn describe(self) -> String {
        match self {
            Dim::NUMBER => "a plain number".to_string(),
            Dim::LENGTH => "a length".to_string(),
            Dim::AREA => "an area".to_string(),
            Dim::VOLUME => "a volume".to_string(),
            Dim::ANGLE => "an angle".to_string(),
            other => other.unit_text("mm"),
        }
    }

    /// Its unit in `length` terms: `mm`, `in²`, `deg`, `mm/deg`.
    pub fn unit_text(self, length: &str) -> String {
        let power = |base: &str, n: i8| match n {
            1 => base.to_string(),
            2 => format!("{base}²"),
            3 => format!("{base}³"),
            n => format!("{base}^{n}"),
        };
        let mut up = Vec::new();
        let mut down = Vec::new();
        for (base, n) in [(length, self.length), ("deg", self.angle)] {
            match n.cmp(&0) {
                std::cmp::Ordering::Greater => up.push(power(base, n)),
                std::cmp::Ordering::Less => down.push(power(base, -n)),
                std::cmp::Ordering::Equal => {}
            }
        }
        match (up.is_empty(), down.is_empty()) {
            (true, true) => String::new(),
            (false, true) => up.join("·"),
            (true, false) => format!("1/{}", down.join("·")),
            (false, false) => format!("{}/{}", up.join("·"), down.join("·")),
        }
    }
}

/// A value and its kind, lengths in millimetres, angles in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub dim: Dim,
}

impl Quantity {
    pub fn new(value: f64, dim: Dim) -> Self {
        Self { value, dim }
    }

    pub fn length(mm: f64) -> Self {
        Self::new(mm, Dim::LENGTH)
    }

    pub fn angle(degrees: f64) -> Self {
        Self::new(degrees, Dim::ANGLE)
    }

    pub fn number(value: f64) -> Self {
        Self::new(value, Dim::NUMBER)
    }

    /// The value in `length`'s terms with its unit: `12.5 mm`, `0.5 in`,
    /// `30 deg`, `3`.
    pub fn display(&self, length: Unit, decimals: usize) -> String {
        let per = mm_per(length).powi(i32::from(self.dim.length));
        let value = trim_number(self.value / per, decimals);
        let unit = self.dim.unit_text(length.short_label());
        if unit.is_empty() {
            value
        } else {
            format!("{value} {unit}")
        }
    }
}

/// Millimetres in one `unit`, exactly (the display table is single
/// precision).
pub fn mm_per(unit: Unit) -> f64 {
    match unit {
        Unit::Mm => 1.0,
        Unit::Cm => 10.0,
        Unit::M => 1000.0,
        Unit::In => 25.4,
        Unit::Ft => 304.8,
    }
}

/// `value` to at most `decimals` places, without trailing zeros.
fn trim_number(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    if text == "-0" { "0".to_string() } else { text }
}

/// What went wrong, and where in the text.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprError {
    pub message: String,
    /// Byte range of the text at fault.
    pub span: Range<usize>,
}

impl ExprError {
    fn new(message: impl Into<String>, span: Range<usize>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExprError {}

/// A reference a formula makes: `object.property`, and where each name is.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub object: String,
    pub property: String,
    pub object_span: Range<usize>,
    pub property_span: Range<usize>,
}

/// What a formula's references read.
pub trait Resolve {
    /// The value of `object.property`, or why there is none.
    fn resolve(&self, object: &str, property: &str) -> Result<Quantity, String>;
}

/// No references at all: every one is an error.
pub struct NoReferences;

impl Resolve for NoReferences {
    fn resolve(&self, object: &str, property: &str) -> Result<Quantity, String> {
        Err(format!("nothing is called {object}.{property}"))
    }
}

/// What evaluating needs beyond the text.
pub struct Context<'a> {
    /// The unit a bare number beside a length is in.
    pub length_unit: Unit,
    pub resolve: &'a dyn Resolve,
}

/// The value of `text`.
pub fn evaluate(text: &str, ctx: &Context) -> Result<Quantity, ExprError> {
    let node = parse(text)?;
    Ok(eval(&node, ctx)?.q)
}

/// The value of `text` for a field that holds `want`: a bare result takes
/// the field's unit, and any other kind is refused. The value is in
/// millimetres or degrees.
pub fn evaluate_as(text: &str, want: Dim, ctx: &Context) -> Result<f64, ExprError> {
    let node = parse(text)?;
    let v = eval(&node, ctx)?;
    let v = adopt(v, want, ctx);
    if v.q.dim != want {
        return Err(ExprError::new(
            format!(
                "this gives {} where {} is needed",
                v.q.dim.describe(),
                want.describe()
            ),
            0..text.len(),
        ));
    }
    Ok(v.q.value)
}

/// Whether `text` parses; the error says where it does not.
pub fn check_syntax(text: &str) -> Result<(), ExprError> {
    parse(text).map(|_| ())
}

/// The references `text` makes, in order.
pub fn references(text: &str) -> Result<Vec<Reference>, ExprError> {
    let node = parse(text)?;
    let mut out = Vec::new();
    collect_references(&node, &mut out);
    Ok(out)
}

/// Whether `text` is a value with no references: a number, a quantity,
/// arithmetic over them.
pub fn is_constant(text: &str) -> bool {
    references(text).is_ok_and(|refs| refs.is_empty())
}

/// `text` with every reference to object `old` naming `new`, or `None`
/// when it has none (or does not parse).
pub fn rename_object(text: &str, old: &str, new: &str) -> Option<String> {
    let refs = references(text).ok()?;
    let spans: Vec<Range<usize>> = refs
        .iter()
        .filter(|r| r.object == old)
        .map(|r| r.object_span.clone())
        .collect();
    replace_spans(text, &spans, &quote_name(new))
}

/// `text` with every reference to `object.old` naming `object.new`.
pub fn rename_property(text: &str, object: &str, old: &str, new: &str) -> Option<String> {
    let refs = references(text).ok()?;
    let spans: Vec<Range<usize>> = refs
        .iter()
        .filter(|r| r.object == object && r.property == old)
        .map(|r| r.property_span.clone())
        .collect();
    replace_spans(text, &spans, &quote_name(new))
}

fn replace_spans(text: &str, spans: &[Range<usize>], with: &str) -> Option<String> {
    if spans.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for span in spans {
        out.push_str(&text[at..span.start]);
        out.push_str(with);
        at = span.end;
    }
    out.push_str(&text[at..]);
    Some(out)
}

/// Whether `name` can be written in a formula as it is.
pub fn is_plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Whether `name` can be written in a formula at all: anything but a
/// backtick, a line break, or nothing.
pub fn is_valid_name(name: &str) -> bool {
    !name.trim().is_empty() && !name.contains(['`', '\n', '\r'])
}

/// `name` as a formula writes it: as it is, or in backticks.
pub fn quote_name(name: &str) -> String {
    if is_plain_name(name) {
        name.to_string()
    } else {
        format!("`{name}`")
    }
}

/// The units a quantity can be written in, with what one is in
/// millimetres or degrees.
pub const UNITS: &[(&str, f64, Dim)] = &[
    ("mm", 1.0, Dim::LENGTH),
    ("cm", 10.0, Dim::LENGTH),
    ("m", 1000.0, Dim::LENGTH),
    ("um", 0.001, Dim::LENGTH),
    ("µm", 0.001, Dim::LENGTH),
    ("in", 25.4, Dim::LENGTH),
    ("ft", 304.8, Dim::LENGTH),
    ("deg", 1.0, Dim::ANGLE),
    ("°", 1.0, Dim::ANGLE),
    ("rad", 180.0 / std::f64::consts::PI, Dim::ANGLE),
];

/// The functions a formula can call, and what each takes, for help and
/// completion.
pub const FUNCTIONS: &[(&str, &str)] = &[
    ("abs", "abs(x): x without its sign"),
    ("min", "min(a, b, ...): the smallest"),
    ("max", "max(a, b, ...): the largest"),
    (
        "round",
        "round(x): to the nearest whole mm, degree or number",
    ),
    ("floor", "floor(x): down to a whole mm, degree or number"),
    ("ceil", "ceil(x): up to a whole mm, degree or number"),
    ("sqrt", "sqrt(x): the square root; of an area, a length"),
    ("hypot", "hypot(a, b): sqrt(a² + b²)"),
    ("sin", "sin(angle)"),
    ("cos", "cos(angle)"),
    ("tan", "tan(angle)"),
    ("asin", "asin(x): an angle"),
    ("acos", "acos(x): an angle"),
    ("atan", "atan(x): an angle"),
    ("atan2", "atan2(y, x): the angle of the point (x, y)"),
    ("if", "if(condition, then, else)"),
];

/// Names that stand alone.
pub const CONSTANTS: &[(&str, f64)] =
    &[("pi", std::f64::consts::PI), ("tau", std::f64::consts::TAU)];

// ---------------------------------------------------------------------------
// Tokens

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Number(f64),
    /// A name, and whether it was written in backticks.
    Name(String, bool),
    Op(&'static str),
    Open,
    Close,
    Comma,
    Dot,
    End,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    span: Range<usize>,
}

const OPERATORS: &[&str] = &[
    "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "%", "^", "<", ">", "!",
];

fn tokenize(text: &str) -> Result<Vec<Token>, ExprError> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let c = rest.chars().next().expect("not at the end");
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        if c.is_ascii_digit() || (c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let mut end = i;
            while end < text.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
                end += 1;
            }
            // An exponent only when digits follow it: `2e3`, `1.5e-2`.
            if end < text.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
                let mut k = end + 1;
                if k < text.len() && (bytes[k] == b'+' || bytes[k] == b'-') {
                    k += 1;
                }
                if k < text.len() && bytes[k].is_ascii_digit() {
                    while k < text.len() && bytes[k].is_ascii_digit() {
                        k += 1;
                    }
                    end = k;
                }
            }
            let value: f64 = text[i..end]
                .parse()
                .map_err(|_| ExprError::new("this is not a number", i..end))?;
            out.push(Token {
                tok: Tok::Number(value),
                span: i..end,
            });
            i = end;
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '°' {
            let mut end = i + c.len_utf8();
            if c != '°' {
                for ch in text[end..].chars() {
                    if ch.is_alphanumeric() || ch == '_' {
                        end += ch.len_utf8();
                    } else {
                        break;
                    }
                }
            }
            out.push(Token {
                tok: Tok::Name(text[i..end].to_string(), false),
                span: i..end,
            });
            i = end;
            continue;
        }
        if c == '`' {
            let Some(close) = text[i + 1..].find('`') else {
                return Err(ExprError::new("this name has no closing `", i..text.len()));
            };
            let name = &text[i + 1..i + 1 + close];
            if !is_valid_name(name) {
                return Err(ExprError::new("an empty name", i..i + close + 2));
            }
            out.push(Token {
                tok: Tok::Name(name.to_string(), true),
                span: i..i + close + 2,
            });
            i += close + 2;
            continue;
        }
        let single = match c {
            '(' => Some(Tok::Open),
            ')' => Some(Tok::Close),
            ',' => Some(Tok::Comma),
            '.' => Some(Tok::Dot),
            _ => None,
        };
        if let Some(tok) = single {
            out.push(Token {
                tok,
                span: i..i + 1,
            });
            i += 1;
            continue;
        }
        let Some(op) = OPERATORS.iter().find(|op| rest.starts_with(**op)) else {
            return Err(ExprError::new(
                format!("`{c}` has no meaning here"),
                start..start + c.len_utf8(),
            ));
        };
        out.push(Token {
            tok: Tok::Op(op),
            span: i..i + op.len(),
        });
        i += op.len();
    }
    out.push(Token {
        tok: Tok::End,
        span: text.len()..text.len(),
    });
    Ok(out)
}

// ---------------------------------------------------------------------------
// Parsing

#[derive(Debug, Clone)]
enum Node {
    /// A number, with the unit written after it (its size and kind).
    Number {
        value: f64,
        unit: Option<(f64, Dim)>,
        span: Range<usize>,
    },
    Constant(f64, Range<usize>),
    /// A formula in brackets.
    Group(Box<Node>, Range<usize>),
    Ref(Reference),
    Unary {
        op: &'static str,
        arg: Box<Node>,
        span: Range<usize>,
    },
    Binary {
        op: &'static str,
        left: Box<Node>,
        right: Box<Node>,
        span: Range<usize>,
    },
    Call {
        name: String,
        args: Vec<Node>,
        span: Range<usize>,
    },
}

impl Node {
    fn span(&self) -> Range<usize> {
        match self {
            Node::Ref(r) => r.object_span.start..r.property_span.end,
            Node::Unary { span, .. }
            | Node::Binary { span, .. }
            | Node::Call { span, .. }
            | Node::Number { span, .. }
            | Node::Constant(_, span)
            | Node::Group(_, span) => span.clone(),
        }
    }
}

fn parse(text: &str) -> Result<Node, ExprError> {
    if text.trim().is_empty() {
        return Err(ExprError::new("empty", 0..0));
    }
    let tokens = tokenize(text)?;
    let mut parser = Parser { tokens, at: 0 };
    let node = parser.expression(0)?;
    let next = parser.peek();
    if next.tok != Tok::End {
        return Err(ExprError::new(
            "nothing should follow here",
            next.span.clone(),
        ));
    }
    Ok(node)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

/// How tightly each operator binds, and whether it groups to the right.
fn binding(op: &str) -> Option<(u8, bool)> {
    Some(match op {
        "||" => (1, false),
        "&&" => (2, false),
        "==" | "!=" | "<" | "<=" | ">" | ">=" => (3, false),
        "+" | "-" => (4, false),
        "*" | "/" | "%" => (5, false),
        "^" => (7, true),
        _ => return None,
    })
}

/// Unary minus and not bind tighter than products and looser than powers:
/// `-2^2` is -4.
const UNARY: u8 = 6;

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.at]
    }

    fn next(&mut self) -> Token {
        let token = self.tokens[self.at].clone();
        if self.at + 1 < self.tokens.len() {
            self.at += 1;
        }
        token
    }

    fn expect(&mut self, tok: Tok, what: &str) -> Result<Token, ExprError> {
        let token = self.next();
        if token.tok == tok {
            Ok(token)
        } else {
            Err(ExprError::new(format!("expected {what} here"), token.span))
        }
    }

    fn expression(&mut self, min: u8) -> Result<Node, ExprError> {
        let mut left = self.unary()?;
        while let Tok::Op(op) = self.peek().tok {
            let Some((power, right_assoc)) = binding(op) else {
                break;
            };
            if power < min {
                break;
            }
            let op_span = self.next().span;
            let right = self.expression(if right_assoc { power } else { power + 1 })?;
            let span = span_of(&left, &op_span).start..span_of(&right, &op_span).end;
            left = Node::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Node, ExprError> {
        if let Tok::Op(op @ ("-" | "+" | "!")) = self.peek().tok {
            let span = self.next().span;
            let arg = self.expression(UNARY)?;
            if op == "+" {
                return Ok(arg);
            }
            let span = span.start..span_of(&arg, &span).end;
            return Ok(Node::Unary {
                op,
                arg: Box::new(arg),
                span,
            });
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, ExprError> {
        let token = self.next();
        match token.tok {
            Tok::Number(value) => {
                // A unit written after it: `2 mm`, `45°`; a name followed by
                // a dot is a reference instead.
                let mut span = token.span.clone();
                let unit = match &self.peek().tok {
                    Tok::Name(name, false)
                        if self.tokens.get(self.at + 1).map(|t| &t.tok) != Some(&Tok::Dot) =>
                    {
                        match UNITS.iter().find(|(u, _, _)| u == name) {
                            Some((_, size, dim)) => {
                                span.end = self.next().span.end;
                                Some((*size, *dim))
                            }
                            None => {
                                let t = self.peek();
                                return Err(ExprError::new(
                                    format!("`{name}` is not a unit"),
                                    t.span.clone(),
                                ));
                            }
                        }
                    }
                    _ => None,
                };
                Ok(Node::Number { value, unit, span })
            }
            Tok::Name(name, quoted) => {
                if self.peek().tok == Tok::Dot {
                    self.next();
                    let property = self.next();
                    let Tok::Name(prop, _) = property.tok else {
                        return Err(ExprError::new(
                            format!("expected a property of {} here", quote_name(&name)),
                            property.span,
                        ));
                    };
                    return Ok(Node::Ref(Reference {
                        object: name,
                        property: prop,
                        object_span: token.span,
                        property_span: property.span,
                    }));
                }
                if !quoted && self.peek().tok == Tok::Open {
                    if !FUNCTIONS.iter().any(|(f, _)| *f == name) {
                        return Err(ExprError::new(
                            format!("there is no function `{name}`"),
                            token.span,
                        ));
                    }
                    self.next();
                    let mut args = Vec::new();
                    if self.peek().tok != Tok::Close {
                        loop {
                            args.push(self.expression(0)?);
                            if self.peek().tok == Tok::Comma {
                                self.next();
                                continue;
                            }
                            break;
                        }
                    }
                    let close = self.expect(Tok::Close, "`)`")?;
                    return Ok(Node::Call {
                        name,
                        args,
                        span: token.span.start..close.span.end,
                    });
                }
                if !quoted && let Some((_, value)) = CONSTANTS.iter().find(|(c, _)| *c == name) {
                    return Ok(Node::Constant(*value, token.span));
                }
                Err(ExprError::new(
                    format!(
                        "`{name}` alone means nothing: name a property, such as {}.length",
                        quote_name(&name)
                    ),
                    token.span,
                ))
            }
            Tok::Open => {
                let inner = self.expression(0)?;
                let close = self.expect(Tok::Close, "`)`")?;
                Ok(Node::Group(
                    Box::new(inner),
                    token.span.start..close.span.end,
                ))
            }
            Tok::End => Err(ExprError::new("the formula ends too soon", token.span)),
            _ => Err(ExprError::new("expected a value here", token.span)),
        }
    }
}

fn span_of(node: &Node, fallback: &Range<usize>) -> Range<usize> {
    let span = node.span();
    if span.is_empty() {
        fallback.clone()
    } else {
        span
    }
}

fn collect_references(node: &Node, out: &mut Vec<Reference>) {
    match node {
        Node::Ref(r) => out.push(r.clone()),
        Node::Unary { arg, .. } => collect_references(arg, out),
        Node::Binary { left, right, .. } => {
            collect_references(left, out);
            collect_references(right, out);
        }
        Node::Call { args, .. } => args.iter().for_each(|a| collect_references(a, out)),
        Node::Group(inner, _) => collect_references(inner, out),
        Node::Number { .. } | Node::Constant(..) => {}
    }
}

// ---------------------------------------------------------------------------
// Evaluation

/// A value while evaluating, and whether it is still a bare number.
#[derive(Debug, Clone, Copy)]
struct Value {
    q: Quantity,
    bare: bool,
}

impl Value {
    fn of(q: Quantity) -> Self {
        Self { q, bare: false }
    }
}

/// A bare number standing for `dim` in `ctx`'s units.
fn adopt(v: Value, dim: Dim, ctx: &Context) -> Value {
    if !v.bare || v.q.dim != Dim::NUMBER || dim == Dim::NUMBER {
        return v;
    }
    let per = mm_per(ctx.length_unit).powi(i32::from(dim.length));
    Value::of(Quantity::new(v.q.value * per, dim))
}

/// Two values made alike for a sum or a comparison: a bare one takes the
/// other's kind; then their kinds must agree.
fn alike(
    a: Value,
    b: Value,
    ctx: &Context,
    span: &Range<usize>,
    what: &str,
) -> Result<(Value, Value), ExprError> {
    let (a, b) = (adopt(a, b.q.dim, ctx), adopt(b, a.q.dim, ctx));
    if a.q.dim != b.q.dim {
        return Err(ExprError::new(
            format!("{what} {} and {}", a.q.dim.describe(), b.q.dim.describe()),
            span.clone(),
        ));
    }
    Ok((a, b))
}

fn finite(value: f64, span: &Range<usize>) -> Result<f64, ExprError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ExprError::new("this has no finite value", span.clone()))
    }
}

fn eval(node: &Node, ctx: &Context) -> Result<Value, ExprError> {
    match node {
        Node::Group(inner, _) => eval(inner, ctx),
        Node::Number {
            value, unit: None, ..
        } => Ok(Value {
            q: Quantity::number(*value),
            bare: true,
        }),
        Node::Number {
            value,
            unit: Some((size, dim)),
            ..
        } => Ok(Value::of(Quantity::new(value * size, *dim))),
        Node::Constant(value, _) => Ok(Value::of(Quantity::number(*value))),
        Node::Ref(r) => ctx
            .resolve
            .resolve(&r.object, &r.property)
            .map(Value::of)
            .map_err(|why| ExprError::new(why, r.object_span.start..r.property_span.end)),
        Node::Unary { op, arg, span } => {
            let v = eval(arg, ctx)?;
            match *op {
                "-" => Ok(Value {
                    q: Quantity::new(-v.q.value, v.q.dim),
                    bare: v.bare,
                }),
                _ => {
                    need_number(v, span, "`!` takes")?;
                    Ok(Value::of(Quantity::number(truth(v.q.value == 0.0))))
                }
            }
        }
        Node::Binary {
            op,
            left,
            right,
            span,
        } => {
            // `&&` and `||` look at the right only when they need to.
            if matches!(*op, "&&" | "||") {
                let l = eval(left, ctx)?;
                need_number(l, span, "`&&` and `||` take")?;
                let l_true = l.q.value != 0.0;
                if (*op == "&&" && !l_true) || (*op == "||" && l_true) {
                    return Ok(Value::of(Quantity::number(truth(l_true))));
                }
                let r = eval(right, ctx)?;
                need_number(r, span, "`&&` and `||` take")?;
                return Ok(Value::of(Quantity::number(truth(r.q.value != 0.0))));
            }
            let l = eval(left, ctx)?;
            let r = eval(right, ctx)?;
            binary(op, l, r, ctx, span)
        }
        Node::Call { name, args, span } => call(name, args, ctx, span),
    }
}

fn truth(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

fn need_number(v: Value, span: &Range<usize>, what: &str) -> Result<(), ExprError> {
    if v.q.dim == Dim::NUMBER {
        Ok(())
    } else {
        Err(ExprError::new(
            format!("{what} plain numbers, not {}", v.q.dim.describe()),
            span.clone(),
        ))
    }
}

fn binary(
    op: &str,
    l: Value,
    r: Value,
    ctx: &Context,
    span: &Range<usize>,
) -> Result<Value, ExprError> {
    match op {
        "+" | "-" | "%" => {
            let (l, r) = alike(l, r, ctx, span, "this adds")?;
            let value = match op {
                "+" => l.q.value + r.q.value,
                "-" => l.q.value - r.q.value,
                _ => {
                    if r.q.value == 0.0 {
                        return Err(ExprError::new(
                            "the remainder of a division by zero",
                            span.clone(),
                        ));
                    }
                    l.q.value.rem_euclid(r.q.value)
                }
            };
            Ok(Value {
                q: Quantity::new(finite(value, span)?, l.q.dim),
                bare: l.bare && r.bare,
            })
        }
        "*" => Ok(Value {
            q: Quantity::new(finite(l.q.value * r.q.value, span)?, l.q.dim.times(r.q.dim)),
            bare: l.bare && r.bare,
        }),
        "/" => {
            if r.q.value == 0.0 {
                return Err(ExprError::new("a division by zero", span.clone()));
            }
            Ok(Value {
                q: Quantity::new(finite(l.q.value / r.q.value, span)?, l.q.dim.over(r.q.dim)),
                bare: l.bare && r.bare,
            })
        }
        "^" => {
            need_number(r, span, "a power takes")?;
            let n = r.q.value;
            let dim = if l.q.dim == Dim::NUMBER {
                Dim::NUMBER
            } else {
                if n.fract() != 0.0 || n.abs() > 9.0 {
                    return Err(ExprError::new(
                        format!("{} can only be raised to a whole power", l.q.dim.describe()),
                        span.clone(),
                    ));
                }
                l.q.dim.power(n as i8)
            };
            Ok(Value {
                q: Quantity::new(finite(l.q.value.powf(n), span)?, dim),
                bare: l.bare,
            })
        }
        _ => {
            let (l, r) = alike(l, r, ctx, span, "this compares")?;
            let (a, b) = (l.q.value, r.q.value);
            let result = match op {
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                ">=" => a >= b,
                "==" => (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0),
                _ => (a - b).abs() > 1e-9 * a.abs().max(b.abs()).max(1.0),
            };
            Ok(Value::of(Quantity::number(truth(result))))
        }
    }
}

fn call(name: &str, args: &[Node], ctx: &Context, span: &Range<usize>) -> Result<Value, ExprError> {
    let count = |n: usize| {
        if args.len() == n {
            Ok(())
        } else {
            Err(ExprError::new(
                format!(
                    "{name} takes {n} value{}, not {}",
                    if n == 1 { "" } else { "s" },
                    args.len()
                ),
                span.clone(),
            ))
        }
    };
    if name == "if" {
        count(3)?;
        let condition = eval(&args[0], ctx)?;
        need_number(condition, span, "a condition is")?;
        // Only the branch taken is worked out: `if(x > 0, a / x, 0)`.
        let taken = if condition.q.value != 0.0 {
            &args[1]
        } else {
            &args[2]
        };
        return eval(taken, ctx);
    }
    let values = args
        .iter()
        .map(|a| eval(a, ctx))
        .collect::<Result<Vec<_>, _>>()?;
    let angle_in = |v: Value| -> Result<f64, ExprError> {
        let v = adopt(v, Dim::ANGLE, ctx);
        if v.q.dim != Dim::ANGLE {
            return Err(ExprError::new(
                format!("{name} takes an angle, not {}", v.q.dim.describe()),
                span.clone(),
            ));
        }
        Ok(v.q.value.to_radians())
    };
    let number_in = |v: Value| -> Result<f64, ExprError> {
        need_number(v, span, &format!("{name} takes"))?;
        Ok(v.q.value)
    };
    let same = |v: Value, value: f64| Value {
        q: Quantity::new(value, v.q.dim),
        bare: v.bare,
    };
    let result = match name {
        "sin" | "cos" | "tan" => {
            count(1)?;
            let a = angle_in(values[0])?;
            let x = match name {
                "sin" => a.sin(),
                "cos" => a.cos(),
                _ => a.tan(),
            };
            Value::of(Quantity::number(x))
        }
        "asin" | "acos" | "atan" => {
            count(1)?;
            let x = number_in(values[0])?;
            if name != "atan" && !(-1.0..=1.0).contains(&x) {
                return Err(ExprError::new(
                    format!("{name} takes a number from -1 to 1"),
                    span.clone(),
                ));
            }
            let a = match name {
                "asin" => x.asin(),
                "acos" => x.acos(),
                _ => x.atan(),
            };
            Value::of(Quantity::angle(a.to_degrees()))
        }
        "atan2" => {
            count(2)?;
            let (y, x) = alike(values[0], values[1], ctx, span, "atan2 takes")?;
            Value::of(Quantity::angle(y.q.value.atan2(x.q.value).to_degrees()))
        }
        "hypot" => {
            count(2)?;
            let (a, b) = alike(values[0], values[1], ctx, span, "hypot takes")?;
            same(a, a.q.value.hypot(b.q.value))
        }
        "sqrt" => {
            count(1)?;
            let v = values[0];
            if v.q.value < 0.0 {
                return Err(ExprError::new(
                    "the square root of a negative",
                    span.clone(),
                ));
            }
            if v.q.dim.length % 2 != 0 || v.q.dim.angle % 2 != 0 {
                return Err(ExprError::new(
                    format!("{} has no square root", v.q.dim.describe()),
                    span.clone(),
                ));
            }
            Value {
                q: Quantity::new(
                    v.q.value.sqrt(),
                    Dim {
                        length: v.q.dim.length / 2,
                        angle: v.q.dim.angle / 2,
                    },
                ),
                bare: v.bare,
            }
        }
        "abs" | "round" | "floor" | "ceil" => {
            count(1)?;
            let v = values[0];
            let x = match name {
                "abs" => v.q.value.abs(),
                "round" => v.q.value.round(),
                "floor" => v.q.value.floor(),
                _ => v.q.value.ceil(),
            };
            same(v, x)
        }
        "min" | "max" => {
            if values.is_empty() {
                return Err(ExprError::new(
                    format!("{name} takes at least one value"),
                    span.clone(),
                ));
            }
            let mut best = values[0];
            for v in &values[1..] {
                let (a, b) = alike(best, *v, ctx, span, &format!("{name} takes"))?;
                let pick_b = if name == "min" {
                    b.q.value < a.q.value
                } else {
                    b.q.value > a.q.value
                };
                best = if pick_b { b } else { a };
            }
            best
        }
        other => {
            return Err(ExprError::new(
                format!("there is no function `{other}`"),
                span.clone(),
            ));
        }
    };
    finite(result.q.value, span)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Table(HashMap<(&'static str, &'static str), Quantity>);

    impl Resolve for Table {
        fn resolve(&self, object: &str, property: &str) -> Result<Quantity, String> {
            self.0
                .iter()
                .find(|((o, p), _)| *o == object && *p == property)
                .map(|(_, q)| *q)
                .ok_or_else(|| format!("nothing is called {object}.{property}"))
        }
    }

    fn table() -> Table {
        Table(HashMap::from([
            (("Printer", "nozzle"), Quantity::length(0.4)),
            (("Printer", "layer"), Quantity::length(0.2)),
            (("Printer", "walls"), Quantity::number(3.0)),
            (("Pad 2", "length"), Quantity::length(12.0)),
            (("Sizes", "tilt"), Quantity::angle(30.0)),
        ]))
    }

    fn eval_in(text: &str, unit: Unit) -> Result<Quantity, ExprError> {
        let t = table();
        evaluate(
            text,
            &Context {
                length_unit: unit,
                resolve: &t,
            },
        )
    }

    fn length(text: &str) -> Result<f64, ExprError> {
        let t = table();
        evaluate_as(
            text,
            Dim::LENGTH,
            &Context {
                length_unit: Unit::Mm,
                resolve: &t,
            },
        )
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn arithmetic_follows_the_usual_order() {
        let n = |t: &str| eval_in(t, Unit::Mm).unwrap().value;
        assert_eq!(n("1 + 2 * 3"), 7.0);
        assert_eq!(n("(1 + 2) * 3"), 9.0);
        assert_eq!(n("2 ^ 3 ^ 2"), 512.0, "powers group to the right");
        assert_eq!(n("-2 ^ 2"), -4.0);
        assert_eq!(n("7 % 3"), 1.0);
        assert_eq!(n("-7 % 3"), 2.0);
        assert_eq!(n("1.5e2 + .5"), 150.5);
        assert!(close(n("2 * pi"), std::f64::consts::TAU));
    }

    #[test]
    fn units_convert_and_kinds_multiply() {
        let q = eval_in("1 in + 1 mm", Unit::Mm).unwrap();
        assert!(close(q.value, 26.4) && q.dim == Dim::LENGTH);
        let area = eval_in("2 mm * 3 cm", Unit::Mm).unwrap();
        assert_eq!((area.value, area.dim), (60.0, Dim::AREA));
        let back = eval_in("sqrt(2 mm * 8 mm)", Unit::Mm).unwrap();
        assert_eq!((back.value, back.dim), (4.0, Dim::LENGTH));
        assert!(close(
            eval_in("1 rad", Unit::Mm).unwrap().value,
            57.295_779_513_082_32
        ));
        assert_eq!(eval_in("45°", Unit::Mm).unwrap(), Quantity::angle(45.0));
        assert_eq!(eval_in("(2 mm)^3", Unit::Mm).unwrap().dim, Dim::VOLUME);
    }

    #[test]
    fn a_bare_number_takes_the_unit_beside_it_or_the_fields() {
        assert_eq!(length("12").unwrap(), 12.0);
        assert_eq!(length("Printer.nozzle + 2").unwrap(), 2.4);
        assert_eq!(
            length("Printer.walls * Printer.nozzle").unwrap(),
            1.2000000000000002
        );
        // In an inch document a bare number beside a length is inches.
        let t = table();
        let inches = Context {
            length_unit: Unit::In,
            resolve: &t,
        };
        assert!(close(evaluate_as("1", Dim::LENGTH, &inches).unwrap(), 25.4));
        assert!(close(
            evaluate_as("Printer.nozzle + 1", Dim::LENGTH, &inches).unwrap(),
            25.8
        ));
        // Angles are degrees.
        let angle = evaluate_as("Sizes.tilt + 15", Dim::ANGLE, &inches).unwrap();
        assert_eq!(angle, 45.0);
    }

    #[test]
    fn a_field_refuses_the_wrong_kind() {
        let err = length("30 deg").unwrap_err();
        assert!(err.message.contains("an angle where a length"), "{err}");
        let err = length("Printer.nozzle * Printer.layer").unwrap_err();
        assert!(err.message.contains("an area"), "{err}");
        let err = length("Printer.nozzle + 30 deg").unwrap_err();
        assert!(err.message.contains("adds a length and an angle"), "{err}");
        assert_eq!(
            &"Printer.nozzle + 30 deg"[err.span.clone()],
            "Printer.nozzle + 30 deg"
        );
    }

    #[test]
    fn functions_check_what_they_take() {
        let n = |t: &str| eval_in(t, Unit::Mm);
        assert!(close(n("sin(30 deg)").unwrap().value, 0.5));
        assert!(close(n("cos(Sizes.tilt * 2)").unwrap().value, 0.5));
        assert!(
            close(n("sin(30)").unwrap().value, 0.5),
            "a bare angle is degrees"
        );
        assert_eq!(n("atan2(1 mm, 1 mm)").unwrap(), Quantity::angle(45.0));
        assert_eq!(n("asin(1)").unwrap(), Quantity::angle(90.0));
        assert_eq!(n("hypot(3 mm, 4 mm)").unwrap(), Quantity::length(5.0));
        assert_eq!(
            n("max(1 mm, 2, Printer.nozzle)").unwrap(),
            Quantity::length(2.0)
        );
        assert_eq!(n("min(3, 2, 5)").unwrap().value, 2.0);
        assert_eq!(n("round(2.6 mm)").unwrap(), Quantity::length(3.0));
        assert!(
            n("sin(2 mm)")
                .unwrap_err()
                .message
                .contains("takes an angle")
        );
        assert!(
            n("sqrt(2 mm)")
                .unwrap_err()
                .message
                .contains("no square root")
        );
        assert!(n("asin(2)").is_err());
        assert!(n("min()").is_err());
        assert!(
            n("hypot(1 mm)")
                .unwrap_err()
                .message
                .contains("takes 2 values")
        );
        assert!(n("nope(1)").unwrap_err().message.contains("no function"));
    }

    #[test]
    fn if_takes_only_its_branch() {
        let q = eval_in("if(Printer.walls > 2, Printer.nozzle * 4, 0)", Unit::Mm).unwrap();
        assert_eq!(q, Quantity::length(1.6));
        // The branch not taken would divide by zero.
        let q = eval_in("if(0 == 1, 1 mm / 0, 2 mm)", Unit::Mm).unwrap();
        assert_eq!(q, Quantity::length(2.0));
        let q = eval_in("if(1 mm < 2 && !(3 < 2) || 0, 1, 2)", Unit::Mm).unwrap();
        assert_eq!(q.value, 1.0);
    }

    #[test]
    fn errors_point_at_what_is_wrong() {
        let at = |t: &str| {
            let err = eval_in(t, Unit::Mm).unwrap_err();
            (t[err.span.clone()].to_string(), err.message)
        };
        assert_eq!(at("2 * Printer.nozle").0, "Printer.nozle");
        assert_eq!(at("1 / (Printer.walls - 3)").0, "1 / (Printer.walls - 3)");
        assert!(at("1 / (Printer.walls - 3)").1.contains("division by zero"));
        assert_eq!(at("2 xy").0, "xy");
        assert_eq!(at("2 + ").1, "the formula ends too soon");
        assert_eq!(at("(1 + 2").1, "expected `)` here");
        assert_eq!(at("1 2").0, "2");
        assert!(at("width * 2").1.contains("alone means nothing"));
        assert!(at("2 $ 3").1.contains("`$`"));
        assert!(eval_in("   ", Unit::Mm).is_err());
    }

    #[test]
    fn names_with_spaces_go_in_backticks() {
        assert_eq!(length("`Pad 2`.length / 2").unwrap(), 6.0);
        assert_eq!(quote_name("Pad 2"), "`Pad 2`");
        assert_eq!(quote_name("Pad_2"), "Pad_2");
        assert!(!is_valid_name("a`b") && !is_valid_name(" "));
        assert!(
            eval_in("`Pad 2.length", Unit::Mm)
                .unwrap_err()
                .message
                .contains("closing")
        );
    }

    #[test]
    fn references_are_listed_and_renamed_where_they_are() {
        let text = "Printer.nozzle*2 + max(Printer.layer, `Pad 2`.length) + nozzle_x.nozzle";
        let refs = references(text).unwrap();
        let names: Vec<(String, String)> = refs
            .iter()
            .map(|r| (r.object.clone(), r.property.clone()))
            .collect();
        assert_eq!(
            names,
            [
                ("Printer".into(), "nozzle".into()),
                ("Printer".into(), "layer".into()),
                ("Pad 2".into(), "length".into()),
                ("nozzle_x".into(), "nozzle".into()),
            ]
        );
        assert_eq!(
            rename_object(text, "Printer", "My printer").unwrap(),
            "`My printer`.nozzle*2 + max(`My printer`.layer, `Pad 2`.length) + nozzle_x.nozzle"
        );
        assert_eq!(
            rename_object(text, "Pad 2", "Base").unwrap(),
            "Printer.nozzle*2 + max(Printer.layer, Base.length) + nozzle_x.nozzle"
        );
        assert_eq!(
            rename_property(text, "Printer", "nozzle", "bore").unwrap(),
            "Printer.bore*2 + max(Printer.layer, `Pad 2`.length) + nozzle_x.nozzle"
        );
        assert_eq!(rename_object(text, "Other", "X"), None);
        assert!(is_constant("2 mm + 3 in * 2") && !is_constant(text));
    }

    #[test]
    fn a_unit_word_before_a_dot_is_a_reference() {
        // An object may be called `m`: `2 m.x` is 2 times m.x, not 2 metres.
        let t = Table(HashMap::from([(("m", "x"), Quantity::length(3.0))]));
        let ctx = Context {
            length_unit: Unit::Mm,
            resolve: &t,
        };
        assert!(
            evaluate("2 m.x", &ctx).is_err(),
            "a number and a value side by side"
        );
        assert_eq!(evaluate("2 * m.x", &ctx).unwrap(), Quantity::length(6.0));
        assert_eq!(evaluate("2 m", &ctx).unwrap(), Quantity::length(2000.0));
    }

    #[test]
    fn quantities_display_in_the_documents_unit() {
        assert_eq!(Quantity::length(25.4).display(Unit::In, 3), "1 in");
        assert_eq!(Quantity::length(12.5).display(Unit::Mm, 3), "12.5 mm");
        assert_eq!(
            Quantity::new(645.16, Dim::AREA).display(Unit::In, 3),
            "1 in²"
        );
        assert_eq!(Quantity::angle(30.0).display(Unit::Mm, 2), "30 deg");
        assert_eq!(Quantity::number(3.0).display(Unit::Mm, 2), "3");
        assert_eq!(
            Dim {
                length: 1,
                angle: -1
            }
            .unit_text("mm"),
            "mm/deg"
        );
    }
}
