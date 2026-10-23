//! Open-set enumerations: a wire string this crate knows decodes to its
//! variant, and any other string decodes to `Other`, kept as it was sent, so
//! a newer peer's value never fails a decode.

/// Define an open-set string enumeration.
///
/// The generated enum is `#[non_exhaustive]`, carries an `Other(String)`
/// variant for values this crate does not know, encodes and decodes as a
/// plain JSON string, and has `parse`, `as_str`, `is_known` and a `KNOWN`
/// list of the values this crate names, in declaration order.
macro_rules! open_set {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $text:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
            /// A value this crate does not name, kept exactly as it was sent.
            Other(String),
        }

        impl $name {
            /// Every value this crate names, in declaration order.
            pub const KNOWN: &'static [&'static str] = &[$($text),+];

            /// The variant for `text`; a value this crate does not name is
            /// [`Self::Other`].
            pub fn parse(text: &str) -> Self {
                match text {
                    $( $text => Self::$variant, )+
                    other => Self::Other(other.to_owned()),
                }
            }

            /// The wire string.
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $text, )+
                    Self::Other(other) => other,
                }
            }

            /// Whether this crate names the value.
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Other(_))
            }
        }

        impl From<&str> for $name {
            fn from(text: &str) -> Self {
                Self::parse(text)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <String as serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::parse(&text))
            }
        }
    };
}
