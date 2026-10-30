//! The shared spellings of runner pair values (CONTRACT §1.2).
//!
//! Validation accepts any token of the right shape, so these lists don't
//! restrict what decodes, and nothing in this crate checks a value against
//! them. They are the spellings the contract requires a runner to declare, the
//! model catalog to serve and the vault to report, so the same value is spelled
//! the same way everywhere and can be matched. A spelling is added here and to
//! the contract's table in the same text release; a test keeps the two equal.

/// `wire_api` values: the wire format of a route, in the contract's table order.
pub const WIRE_APIS: &[&str] = &[
    "anthropic.messages",
    "openai.responses",
    "openai.chat_completions",
    "openai.chatgpt_responses",
    "google.generate_content",
    "google.code_assist",
    "aws.bedrock_converse",
];

/// `auth_method` values: the vault's names for its credential kinds, in the
/// contract's table order.
pub const AUTH_METHODS: &[&str] = &["apikey", "oauth", "chatgpt", "antigravity"];

#[cfg(test)]
mod tests {
    use super::{AUTH_METHODS, WIRE_APIS};
    use crate::declarations::is_runner_token;

    /// The tokens in the first column of the contract's table whose header
    /// cell is `header`, in order.
    fn contract_column(header: &str) -> Vec<String> {
        let contract = include_str!("../CONTRACT.md");
        let mut rows = contract
            .lines()
            .map(str::trim)
            .skip_while(|line| !line.starts_with(&format!("| {header} |")))
            .skip(2);
        let mut tokens = Vec::new();
        while let Some(line) = rows.next().filter(|line| line.starts_with('|')) {
            let cell = line.split('|').nth(1).unwrap().trim();
            tokens.push(cell.trim_matches('`').to_string());
        }
        tokens
    }

    #[test]
    fn every_shared_spelling_is_a_valid_token() {
        for token in WIRE_APIS.iter().chain(AUTH_METHODS) {
            assert!(
                is_runner_token(token),
                "{token} fails the pair token grammar"
            );
        }
    }

    #[test]
    fn the_constants_match_the_contract_tables() {
        assert_eq!(contract_column("`wire_api`"), WIRE_APIS);
        assert_eq!(contract_column("`auth_method`"), AUTH_METHODS);
    }
}
