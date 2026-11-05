//! Closed billing classifications shared by model-choice answers.

use serde::{Deserialize, Serialize};

/// How a model is billed. Absence of this value means billing is unknown.
/// Unknown wire values make an answer malformed, rather than becoming unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Billing {
    Subscription,
    PerToken,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_is_a_closed_wire_set() {
        for (value, wire) in [
            (Billing::Subscription, "\"subscription\""),
            (Billing::PerToken, "\"per_token\""),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Billing>(wire).unwrap(), value);
        }
        assert!(serde_json::from_str::<Billing>("\"free\"").is_err());
    }
}
