use kdl::KdlValue;

use crate::{
    emit::{is_bare, quote},
    reference::{Reference, ReferenceError},
    span::Span,
};

/// A value in argument or property position.
///
/// The one distinction KDL itself does not keep is the one stackup gives meaning to: a **quoted
/// string** is an expression or literal text (`"100nF"`, `"1 / value"`, `"Device:R"`), while a
/// **bare identifier** is a name — a reference to something in scope, or a member of an enumerated
/// type (`value=ra`, `add0=gnd`, `type=power`). So they are two variants here, and the writer
/// keeps them apart on the way out.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A quoted string.
    String(String),
    /// A bare identifier.
    Name(String),
    Integer(i128),
    Float(f64),
    Bool(bool),
    Null,
}

impl Value {
    /// The text of a string or a name; `None` for a number, boolean or null.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::Name(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_quoted(&self) -> bool {
        matches!(self, Value::String(_))
    }

    /// The value read as a reference. A string or a name parses; anything else is an error.
    pub fn as_reference(&self) -> Result<Reference, ReferenceError> {
        match self.as_str() {
            Some(s) => Reference::parse(s),
            None => Err(ReferenceError {
                message: format!("{} is not a reference", self.describe()),
                at: 0,
            }),
        }
    }

    /// A short description for a message: "a number", "a boolean", …
    pub fn describe(&self) -> &'static str {
        match self {
            Value::String(_) => "a string",
            Value::Name(_) => "a name",
            Value::Integer(_) | Value::Float(_) => "a number",
            Value::Bool(_) => "a boolean",
            Value::Null => "null",
        }
    }

    /// How the value is written in KDL.
    pub fn repr(&self) -> String {
        match self {
            Value::String(s) => quote(s),
            Value::Name(n) => {
                if is_bare(n) {
                    n.clone()
                } else {
                    quote(n)
                }
            }
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => {
                if f.is_finite() {
                    format!("{f:?}")
                } else if f.is_nan() {
                    "#nan".into()
                } else if *f > 0.0 {
                    "#inf".into()
                } else {
                    "#-inf".into()
                }
            }
            Value::Bool(b) => format!("#{b}"),
            Value::Null => "#null".into(),
        }
    }

    pub(crate) fn to_kdl(&self) -> KdlValue {
        match self {
            Value::String(s) | Value::Name(s) => KdlValue::String(s.clone()),
            Value::Integer(i) => KdlValue::Integer(*i),
            Value::Float(f) => KdlValue::Float(*f),
            Value::Bool(b) => KdlValue::Bool(*b),
            Value::Null => KdlValue::Null,
        }
    }
}

impl From<&str> for Value {
    /// A quoted string.
    fn from(s: &str) -> Self {
        Value::String(s.into())
    }
}

impl From<i128> for Value {
    fn from(i: i128) -> Self {
        Value::Integer(i)
    }
}

/// A `key=value` property on a statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub key: String,
    pub value: Value,
    pub span: Span,
}

impl Property {
    pub fn new(key: impl Into<String>, value: impl Into<Value>) -> Self {
        Property {
            key: key.into(),
            value: value.into(),
            span: Span::default(),
        }
    }

    /// A property whose value is a bare name.
    pub fn name(key: impl Into<String>, name: impl Into<String>) -> Self {
        Property {
            key: key.into(),
            value: Value::Name(name.into()),
            span: Span::default(),
        }
    }
}

/// Finds a property by key.
pub fn property<'a>(props: &'a [Property], key: &str) -> Option<&'a Property> {
    props.iter().find(|p| p.key == key)
}
