//! String value sets that a newer provider may extend.
//!
//! Each set decodes a value it does not know as `Other`, keeping the exact
//! string, so an older reader never fails on a newer provider's value and
//! re-encodes it unchanged.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! open_set {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $wire:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )+
            /// A value this version of the contract does not know, kept
            /// exactly as received. Build values from strings with
            /// `from_wire`, so a known string never lands here.
            Other(String),
        }

        impl $name {
            /// Every value this version knows, as sent on the wire.
            pub const KNOWN: &'static [&'static str] = &[$($wire),+];

            /// Decodes a wire string; an unknown string becomes `Other`.
            pub fn from_wire(value: &str) -> Self {
                match value {
                    $( $wire => Self::$variant, )+
                    other => Self::Other(other.to_owned()),
                }
            }

            /// The wire string.
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $wire, )+
                    Self::Other(other) => other,
                }
            }

            /// Whether this version of the contract knows the value.
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Other(_))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Ok(Self::from_wire(&value))
            }
        }
    };
}

open_set! {
    /// A step's prompt-cache verdict. The rules that choose it are in
    /// `CONTRACT.md` and are computed over the step's whole session in time
    /// order.
    ///
    /// `Unknown` is a real verdict: the cache's health cannot be judged for
    /// this step. `Other` is a value this version does not know (a newer
    /// verdict); show it as received rather than treating it as `Unknown`.
    pub enum Verdict {
        /// The session's first step.
        ColdStart => "cold_start",
        /// Retention of at least 0.95, or no usable earlier step to compare.
        Stable => "stable",
        /// Retention of at least 0.80 and below 0.95.
        Warning => "warning",
        /// Retention below 0.80 with a positive cache read.
        Bust => "bust",
        /// A cache read of zero.
        FullBust => "full_bust",
        /// The cache's health cannot be judged for this step.
        Unknown => "unknown",
    }
}

open_set! {
    /// Where the provider learned of a step.
    pub enum StepSource {
        /// Read from a harness store or a runner's durable log.
        File => "file",
        /// Known so far only from a runner's bus event.
        Runner => "runner",
    }
}

open_set! {
    /// On a prompt drop, whether the request body shrank compared with the
    /// earlier step.
    pub enum RequestSize {
        /// The current request body is shorter than the earlier step's.
        Smaller => "smaller",
        /// The current request body is not shorter.
        NotSmaller => "not_smaller",
    }
}
