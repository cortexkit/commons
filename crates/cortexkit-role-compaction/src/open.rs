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
macro_rules! open_tagged_value {
    ($name:ident, $variant:ident, $value:ident) => {
        Ok(Self::$variant)
    };
    ($name:ident, $variant:ident, $value:ident, $payload:ident: $inner:ty) => {
        crate::wire::tagged_fields::<$inner, D::Error>($value).map(Self::$variant)
    };
    ($name:ident, $variant:ident, $value:ident, { $($(#[$attr:meta])* $field:ident: $ty:ty),* }) => {{
        #[derive(serde::Deserialize)]
        struct Fields {
            $($(#[$attr])* $field: $ty),*
        }
        let fields: Fields = crate::wire::tagged_fields::<_, D::Error>($value)?;
        Ok(Self::$variant { $($field: fields.$field),* })
    }};
}

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
                let value = serde_json::Value::deserialize(deserializer)?;
                let name = value.get($tag).and_then(serde_json::Value::as_str)
                    .ok_or_else(|| serde::de::Error::custom(concat!("expected string tag `", $tag, "`")))?;
                match name {
                    $($wire => open_tagged_value!($name, $variant, value
                        $(, $payload: $inner)?
                        $(, { $($(#[$fattr])* $field: $fty),* })?
                    ),)*
                    _ => {
                        let name = name.to_owned();
                        let mut fields = value.as_object().expect("tagged object").clone();
                        fields.remove($tag);
                        Ok(Self::Unknown { name, fields })
                    }
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    open_tagged! {
        /// A reader that knows only an older refusal code still preserves new ones.
        #[derive(Debug, PartialEq, Eq)]
        pub enum OlderViewRefusal, "code" {
            DuplicateCut => "duplicate_cut",
        }
    }

    #[test]
    fn older_reader_preserves_cut_before_move_as_unknown() {
        let raw = json!({"code":"cut_before_move","moved_through":"m"});
        let decoded: OlderViewRefusal = serde_json::from_value(raw.clone()).unwrap();
        assert!(matches!(
            &decoded,
            OlderViewRefusal::Unknown { name, fields }
                if name == "cut_before_move" && fields.get("moved_through") == Some(&json!("m"))
        ));
        assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
    }

    #[test]
    fn older_reader_preserves_render_failed_as_unknown() {
        for raw in [
            json!({"code":"render_failed"}),
            json!({"code":"render_failed", "mid":"m"}),
            json!({"code":"render_failed", "mid":"m", "part":"p"}),
            json!({"code":"render_failed", "mid":"m", "part":"p", "field":"output"}),
        ] {
            let decoded: OlderViewRefusal = serde_json::from_value(raw.clone()).unwrap();
            assert!(matches!(
                &decoded,
                OlderViewRefusal::Unknown { name, .. } if name == "render_failed"
            ));
            assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
        }
    }
}
