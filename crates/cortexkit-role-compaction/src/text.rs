//! Shared, immutable UTF-8 text for the content fields of the part model.
//!
//! A part's text, reasoning, tool output and delivered text can be large and
//! are held by several structures at once: the runner's own message, the
//! entry built from it, a compact copy, a render plan. [`Text`] lets all of
//! them refer to one allocation. Cloning a `Text` copies a pointer, never the
//! content.
//!
//! On the wire a `Text` is a JSON string, exactly as a `String` is, and it
//! compares, hashes and orders by its content, as `str` does.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Shared, immutable UTF-8 text. See the [module documentation](self).
#[derive(Clone)]
pub struct Text(Arc<str>);

impl Text {
    /// The text, copied into a new shared buffer.
    pub fn new(text: &str) -> Self {
        Self(Arc::from(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::new("")
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

// Equality, hashing and ordering follow the content, exactly as `str`'s do,
// so borrowing as `str` keeps map lookups consistent.
impl Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<Arc<str>> for Text {
    /// The buffer, shared, not copied.
    fn from(shared: Arc<str>) -> Self {
        Self(shared)
    }
}

impl From<&Arc<str>> for Text {
    /// The buffer, shared, not copied.
    fn from(shared: &Arc<str>) -> Self {
        Self(Arc::clone(shared))
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        Self(Arc::from(text))
    }
}

impl From<&String> for Text {
    fn from(text: &String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<Box<str>> for Text {
    fn from(text: Box<str>) -> Self {
        Self(Arc::from(text))
    }
}

impl From<Text> for String {
    fn from(text: Text) -> Self {
        text.as_str().to_owned()
    }
}

impl From<&Text> for String {
    fn from(text: &Text) -> Self {
        text.as_str().to_owned()
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Text {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

macro_rules! text_eq {
    ($($other:ty),*) => {$(
        impl PartialEq<$other> for Text {
            fn eq(&self, other: &$other) -> bool {
                self.as_str() == AsRef::<str>::as_ref(other)
            }
        }

        impl PartialEq<Text> for $other {
            fn eq(&self, other: &Text) -> bool {
                AsRef::<str>::as_ref(self) == other.as_str()
            }
        }
    )*};
}

text_eq!(str, &str, String);

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl Serialize for Text {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Text;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }

            // Straight into the shared buffer: one allocation, no
            // intermediate `String`.
            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Text, E> {
                Ok(Text::new(text))
            }

            fn visit_string<E: serde::de::Error>(self, text: String) -> Result<Text, E> {
                Ok(Text::from(text))
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn a_shared_buffer_is_not_copied() {
        let whole: Arc<str> = Arc::from("héllo world");
        let text = Text::from(whole.clone());
        assert_eq!(text.as_ptr(), whole.as_ptr());
        assert_eq!(text, "héllo world");
    }

    #[test]
    fn a_clone_shares_the_buffer() {
        let text = Text::from(String::from("héllo world"));
        assert_eq!(text.clone().as_ptr(), text.as_ptr());
        assert_eq!(std::mem::size_of::<Text>(), 16);
    }

    #[test]
    fn equality_order_and_hash_follow_the_content() {
        let first = Text::from("abc");
        let second = Text::from(Arc::<str>::from("abc"));
        let copied = Text::from(String::from("abc"));
        assert_eq!(first, second);
        assert_eq!(first, copied);
        assert_eq!(hash_of(&first), hash_of("abc"));
        assert_eq!(hash_of(&second), hash_of(&copied));
        assert_eq!(Text::from("abd").cmp(&first), std::cmp::Ordering::Greater);
        assert_eq!(Text::from("ab").cmp(&first), std::cmp::Ordering::Less);
        assert_eq!(first, String::from("abc"));
        assert_eq!("abc", first);
    }

    #[test]
    fn it_travels_as_a_json_string() {
        let text = Text::from("line\n\"quoted\" é");
        let wire = serde_json::to_string(&text).unwrap();
        assert_eq!(
            wire,
            serde_json::to_string(&String::from("line\n\"quoted\" é")).unwrap()
        );
        assert_eq!(serde_json::from_str::<Text>(&wire).unwrap(), text);
        let value = serde_json::to_value(&text).unwrap();
        assert_eq!(serde_json::from_value::<Text>(value).unwrap(), text);
        assert!(serde_json::from_str::<Text>("1").is_err());
    }
}
