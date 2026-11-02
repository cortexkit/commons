// String enums keep values they don't recognise, as the runner contract's do.
macro_rules! open_string {
    ($name:ident { $($variant:ident => $wire:literal),* $(,)? }) => {
        impl $name {
            pub fn parse(value: &str) -> Self {
                match value {
                    $($wire => Self::$variant,)*
                    value => Self::Unknown(value.to_owned()),
                }
            }

            /// The wire spelling, including an unrecognized value.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)*
                    Self::Unknown(value) => value,
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self { Self::parse(&value) }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self { value.as_str().to_owned() }
        }
    };
}

// A tagged enum must distinguish a later tag from a malformed known variant:
// only the former may be preserved as Unknown. An untagged catch-all alone
// would silently accept missing required fields on known requests.
macro_rules! open_tagged {
    (
        $(#[$attr:meta])*
        pub enum $name:ident, $tag:literal {
            $(
                $(#[$vattr:meta])*
                $variant:ident => $wire:literal
                $(($payload:ident: $inner:ty))?
                $({ $($(#[$fattr:meta])* $field:ident: $fty:ty),* $(,)? })?
            ),* $(,)?
        }
    ) => {
        $(#[$attr])*
        #[derive(serde::Serialize)]
        #[serde(tag = $tag)]
        #[non_exhaustive]
        pub enum $name {
            $(
                $(#[$vattr])*
                #[serde(rename = $wire)]
                $variant $(($inner))? $({ $($(#[$fattr])* $field: $fty),* })?,
            )*
            /// An unrecognised tag and its fields, kept as JSON without
            /// interpretation. Decoding it implies no support for its meaning:
            /// receivers handle it as CONTRACT.md W1.5 says.
            #[serde(untagged)]
            Unknown {
                #[serde(rename = $tag)]
                name: String,
                #[serde(flatten)]
                fields: serde_json::Map<String, serde_json::Value>,
            },
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(tag = $tag)]
                enum Known {
                    $(
                        #[serde(rename = $wire)]
                        $variant $(($inner))? $({ $($(#[$fattr])* $field: $fty),* })?,
                    )*
                }
                let value = serde_json::Value::deserialize(deserializer)?;
                let name = value.get($tag).and_then(serde_json::Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom(concat!("expected string tag `", $tag, "`")))?;
                if matches!(name, $($wire)|*) {
                    let known: Known = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                    Ok(match known {
                        $(Known::$variant $(($payload))? $({ $($field),* })? =>
                            Self::$variant $(($payload))? $({ $($field),* })?,)*
                    })
                } else {
                    let name = name.to_owned();
                    let mut fields = value.as_object().expect("tagged object").clone();
                    fields.remove($tag);
                    Ok(Self::Unknown { name, fields })
                }
            }
        }
    };
}
