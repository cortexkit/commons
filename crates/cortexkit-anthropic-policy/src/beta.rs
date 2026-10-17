use std::{error::Error, fmt};

/// A beta line was not an ASCII HTTP field value. No source bytes are retained:
/// callers can report refusal without accidentally logging inbound header data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidBetaHeader;

impl fmt::Display for InvalidBetaHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Anthropic beta header is not a valid ASCII header value")
    }
}

impl Error for InvalidBetaHeader {}

fn validate(value: &str) -> Result<(), InvalidBetaHeader> {
    if value
        .bytes()
        .all(|byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
    {
        Ok(())
    } else {
        Err(InvalidBetaHeader)
    }
}

/// Stable union across all lines: split commas, trim, discard empty tokens and
/// deduplicate exact (case-sensitive) tokens in first-seen order. Validate whole
/// lines before splitting so trimming cannot hide an invalid byte.
pub fn merge_beta_headers<S: AsRef<str>>(
    values: impl IntoIterator<Item = S>,
) -> Result<String, InvalidBetaHeader> {
    let mut tokens = Vec::new();
    for value in values {
        validate(value.as_ref())?;
        for token in value
            .as_ref()
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            if !tokens.iter().any(|seen| seen == token) {
                tokens.push(token.to_owned());
            }
        }
    }
    Ok(tokens.join(","))
}

/// Remove only the exact token, retaining every unrelated beta in stable order.
/// Validation also applies to removed tokens' lines, so removal cannot conceal
/// invalid input. An empty result means the client can omit the beta field.
pub fn remove_beta_token<S: AsRef<str>>(
    values: impl IntoIterator<Item = S>,
    remove: &str,
) -> Result<String, InvalidBetaHeader> {
    validate(remove)?;
    let merged = merge_beta_headers(values)?;
    Ok(merged
        .split(',')
        .filter(|token| *token != remove)
        .collect::<Vec<_>>()
        .join(","))
}
