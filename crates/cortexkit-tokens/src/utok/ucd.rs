//! cortexkit-tokens: the Unicode lookups the tokenizer needs, on stable Rust.
//!
//! Upstream reads these from the `xutf` crate, which only builds on nightly
//! Rust. This module gives the same names over the stable `unicode-*` crates,
//! pinned to Unicode 16.0.0, the data generation upstream pins `xutf` to and
//! the reference tokenizers' regex tables use. A different generation can
//! change how a character newly added to Unicode is classified, which would
//! change counts, so the test below fails if any dependency's Unicode data
//! version changes.

use unicode_properties::UnicodeGeneralCategory;
pub use unicode_properties::{GeneralCategory, GeneralCategoryGroup};
use unicode_script::{Script, UnicodeScript};

/// Unicode data generation of every lookup here.
#[cfg(test)]
pub const UCD_VERSION: (u8, u8, u8) = unicode_normalization::UNICODE_VERSION;

/// `General_Category` and `Script` of a scalar value.
pub trait Ucd: Copy {
    fn general_category(self) -> GeneralCategory;

    fn general_category_group(self) -> GeneralCategoryGroup {
        group(self.general_category())
    }

    fn script(self) -> Script;
}

impl Ucd for char {
    fn general_category(self) -> GeneralCategory {
        UnicodeGeneralCategory::general_category(self)
    }

    fn script(self) -> Script {
        UnicodeScript::script(&self)
    }
}

/// A value that is not a scalar value (a surrogate or past U+10FFFF) is
/// unassigned and has no script, as upstream treats it.
impl Ucd for u32 {
    fn general_category(self) -> GeneralCategory {
        char::from_u32(self).map_or(GeneralCategory::Unassigned, Ucd::general_category)
    }

    fn script(self) -> Script {
        char::from_u32(self).map_or(Script::Unknown, Ucd::script)
    }
}

/// The group a category belongs to.
pub fn group(category: GeneralCategory) -> GeneralCategoryGroup {
    use GeneralCategory as C;
    use GeneralCategoryGroup as G;
    match category {
        C::UppercaseLetter
        | C::LowercaseLetter
        | C::TitlecaseLetter
        | C::ModifierLetter
        | C::OtherLetter => G::Letter,
        C::NonspacingMark | C::SpacingMark | C::EnclosingMark => G::Mark,
        C::DecimalNumber | C::LetterNumber | C::OtherNumber => G::Number,
        C::ConnectorPunctuation
        | C::DashPunctuation
        | C::OpenPunctuation
        | C::ClosePunctuation
        | C::InitialPunctuation
        | C::FinalPunctuation
        | C::OtherPunctuation => G::Punctuation,
        C::MathSymbol | C::CurrencySymbol | C::ModifierSymbol | C::OtherSymbol => G::Symbol,
        C::SpaceSeparator | C::LineSeparator | C::ParagraphSeparator => G::Separator,
        C::Control | C::Format | C::Surrogate | C::PrivateUse | C::Unassigned => G::Other,
    }
}

/// Upstream calls `category.group()`; this gives categories that method.
pub trait CategoryGroup {
    fn group(self) -> GeneralCategoryGroup;
}

impl CategoryGroup for GeneralCategory {
    fn group(self) -> GeneralCategoryGroup {
        group(self)
    }
}

/// Canonical combining class of a scalar value; 0 for anything else.
pub fn canonical_combining_class(cp: u32) -> u8 {
    char::from_u32(cp).map_or(0, unicode_normalization::char::canonical_combining_class)
}

/// Whether `text` is certainly NFC already. `false` means normalizing may
/// change it, so callers keep their borrowed fast path only on `true`.
pub fn is_nfc(text: &str) -> bool {
    unicode_normalization::is_nfc_quick(text.chars()) == unicode_normalization::IsNormalized::Yes
}

/// [`is_nfc`] over raw code points. A value that is not a scalar value
/// normalizes to itself under the permissive decoding the tokenizer uses.
pub fn is_nfc_codepoints(codepoints: impl IntoIterator<Item = u32>) -> bool {
    let chars = codepoints
        .into_iter()
        .map(|cp| char::from_u32(cp).unwrap_or('\u{fffd}'));
    unicode_normalization::is_nfc_quick(chars) == unicode_normalization::IsNormalized::Yes
}

/// NFC of a borrowed string.
pub trait ToUnicodeNormalized {
    fn to_nfc(&self) -> String;
}

impl ToUnicodeNormalized for str {
    fn to_nfc(&self) -> String {
        unicode_normalization::UnicodeNormalization::nfc(self).collect()
    }
}

/// NFC of an owned string.
pub trait IntoUnicodeNormalized {
    fn into_nfc(self) -> String;
}

impl IntoUnicodeNormalized for String {
    fn into_nfc(self) -> String {
        if is_nfc(&self) {
            return self;
        }
        unicode_normalization::UnicodeNormalization::nfc(self.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_lookup_is_on_unicode_16() {
        assert_eq!(super::UCD_VERSION, (16, 0, 0));
        assert_eq!(unicode_properties::UNICODE_VERSION, (16, 0, 0));
        assert_eq!(unicode_script::UNICODE_VERSION, (16, 0, 0));
    }
}
