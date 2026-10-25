//! The values a caller can hand a template: a small typed tree, plus
//! [`TextValue`], which is its text when printed and a map when a template
//! reads a name from it.

use std::{collections::BTreeMap, fmt, sync::Arc};

use minijinja::value::{Enumerator, Object, ObjectRepr};
use minijinja::Value as MinijinjaValue;

/// A value a caller supplies to templates as a root (see
/// [`RenderContext::with_root`](crate::RenderContext::with_root)) or as a field
/// of a [`TextValue`].
///
/// Each variant renders exactly as MiniJinja renders the value it holds today:
/// a string as its characters, an integer as decimal digits (`6`), a boolean
/// as `true` or `false`, a list as iterable and indexable, and a map as a
/// collection of keys and values. A [`TextValue`](Value::Text) prints its own
/// text. No line ending is ever rewritten: whatever the caller passes is
/// rendered as given.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// Text, rendered as its characters and indexed as bytes.
    String(String),
    /// An integer, rendered in decimal.
    Integer(i64),
    /// A boolean, rendered as `true` or `false`.
    Boolean(bool),
    /// A list, rendered as MiniJinja renders a list.
    List(Vec<Value>),
    /// A map from keys to values. Keys are sorted, so the same map always
    /// renders the same bytes.
    Map(BTreeMap<String, Value>),
    /// Text that also answers named field reads; see [`TextValue`].
    Text(TextValue),
}

impl Value {
    /// Text.
    pub fn string(text: impl Into<String>) -> Self {
        Value::String(text.into())
    }

    /// An integer.
    pub fn integer(number: i64) -> Self {
        Value::Integer(number)
    }

    /// A boolean.
    pub fn boolean(flag: bool) -> Self {
        Value::Boolean(flag)
    }

    /// A list of the given values.
    pub fn list(items: impl IntoIterator<Item = Value>) -> Self {
        Value::List(items.into_iter().collect())
    }

    /// A map of the given key/value pairs.
    pub fn map(pairs: impl IntoIterator<Item = (impl Into<String>, Value)>) -> Self {
        Value::Map(
            pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    /// Text that also answers named field reads.
    pub fn text(text: &str) -> Self {
        Value::Text(TextValue::new(text))
    }

    /// The value as MiniJinja sees it.
    pub(crate) fn to_minijinja(&self) -> MinijinjaValue {
        match self {
            Value::String(text) => MinijinjaValue::from(text.as_str()),
            Value::Integer(number) => MinijinjaValue::from(*number),
            Value::Boolean(flag) => MinijinjaValue::from(*flag),
            Value::List(items) => MinijinjaValue::from_iter(items.iter().map(Value::to_minijinja)),
            Value::Map(entries) => MinijinjaValue::from_iter(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_minijinja())),
            ),
            Value::Text(text) => MinijinjaValue::from_object(text.clone()),
        }
    }
}

impl fmt::Display for Value {
    /// The text MiniJinja prints for this value, so a caller can log or
    /// compare a value without rendering a template.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_minijinja())
    }
}

/// Text that a template prints verbatim while it can also read named fields
/// from it: `{{ contract }}` renders the text itself, and
/// `contract.submit_tool` renders the value of that field.
///
/// A name that was never added is undefined, so reading it is a strict
/// undefined error, exactly as reading an unknown name of any other value is.
///
/// ```
/// use cortexkit_persona::{RenderContext, TextValue, Value};
///
/// let contract = TextValue::new("Answer in one line.").with_field("submit_tool", true);
/// let _value = Value::Text(contract);
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TextValue {
    text: String,
    fields: BTreeMap<String, Value>,
}

impl TextValue {
    /// Text with no fields.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fields: BTreeMap::new(),
        }
    }

    /// The text as given.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The fields, by name.
    pub fn fields(&self) -> &BTreeMap<String, Value> {
        &self.fields
    }

    /// Builder form of [`Self::with_field`].
    pub fn field(mut self, name: &str, value: Value) -> Self {
        self.fields.insert(name.to_string(), value);
        self
    }

    /// Add a field that a template reads as `name`. A later call with the same
    /// name replaces the earlier value.
    pub fn with_field(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(name.to_string(), value.into());
        self
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::String(text.to_string())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::String(text)
    }
}

impl From<bool> for Value {
    fn from(flag: bool) -> Self {
        Value::Boolean(flag)
    }
}

macro_rules! integer_value {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for Value {
                fn from(number: $ty) -> Self {
                    Value::Integer(i64::from(number))
                }
            }
        )*
    };
}

integer_value!(i8, i16, i32, u8, u16, u32, i64);

impl From<TextValue> for Value {
    fn from(text: TextValue) -> Self {
        Value::Text(text)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(items: Vec<T>) -> Self {
        Value::List(items.into_iter().map(Into::into).collect())
    }
}

impl<const N: usize, T: Into<Value>> From<[T; N]> for Value {
    fn from(items: [T; N]) -> Self {
        Value::List(items.into_iter().map(Into::into).collect())
    }
}

/// The object MiniJinja holds for a [`TextValue`]. Its repr is `Plain`, so
/// printing, string conversion and serialisation all produce the text itself;
/// named reads still go through `get_value`.
impl Object for TextValue {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Plain
    }

    fn get_value(self: &Arc<Self>, key: &MinijinjaValue) -> Option<MinijinjaValue> {
        let name = key.as_str()?;
        self.fields.get(name).map(Value::to_minijinja)
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Values(
            self.fields
                .keys()
                .map(|key| MinijinjaValue::from(key.as_str()))
                .collect(),
        )
    }

    fn render(self: &Arc<Self>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(value: &Value) -> String {
        value.to_minijinja().to_string()
    }

    #[test]
    fn each_kind_renders_as_minijinja_renders_that_value() {
        assert_eq!(rendered(&Value::string("a\r\nb")), "a\r\nb");
        assert_eq!(rendered(&Value::integer(6)), "6");
        assert_eq!(rendered(&Value::from(-12i32)), "-12");
        assert_eq!(rendered(&Value::boolean(true)), "true");
        assert_eq!(rendered(&Value::boolean(false)), "false");
        assert_eq!(
            rendered(&Value::list([Value::integer(1), Value::string("x")])),
            "[1, \"x\"]"
        );
        assert_eq!(
            rendered(&Value::map([
                ("b", Value::integer(2)),
                ("a", Value::integer(1))
            ])),
            "{\"a\": 1, \"b\": 2}"
        );
    }

    #[test]
    fn a_text_value_renders_its_text_and_answers_its_fields() {
        let text = TextValue::new("line one\r\nline two")
            .with_field("submit_tool", true)
            .with_field("count", 2i64);
        let value = Value::Text(text);
        assert_eq!(rendered(&value), "line one\r\nline two");
        let minijinja = value.to_minijinja();
        assert_eq!(
            minijinja.get_attr("submit_tool").unwrap().to_string(),
            "true"
        );
        assert_eq!(minijinja.get_attr("count").unwrap().to_string(), "2");
        assert!(minijinja.get_attr("absent").unwrap().is_undefined());
    }
}
