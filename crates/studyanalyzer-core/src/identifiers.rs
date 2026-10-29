//! Validation and canonical forms of the identifiers a paper can be reached
//! by: PMID (or a preprint accession), DOI and PMC ID. Every value that leaves
//! this module is already canonical, so two spellings of one paper compare equal.

use std::fmt;

use bmlib::fulltext::service::normalise_pmc_id;
use bmlib::publications::retractions::canonical_pmid;
use bmlib::publications::storage::normalize_doi;
use serde::Serialize;

use crate::config::{
    DOI_NAMESPACE, DOI_REGISTRANT_MAX_DIGITS, DOI_REGISTRANT_MIN_DIGITS, MAX_IDENTIFIER_CHARS,
    PMC_PREFIX, PREPRINT_PREFIX,
};

/// The canonical identifiers of one paper. Each present field is already in
/// canonical form; an absent identifier is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Identifiers {
    /// The PubMed ID (digits, no leading zeros) or a preprint accession (`PPR` + digits).
    pub pmid: Option<String>,
    /// The DOI in lower case, without any `doi:` or URL prefix.
    pub doi: Option<String>,
    /// The PubMed Central ID, always with its `PMC` prefix.
    pub pmc_id: Option<String>,
}

/// Why an identifier was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifierError {
    /// Nothing usable was given: every field was blank or absent.
    Empty,
    /// The value had this many characters, more than `MAX_IDENTIFIER_CHARS`.
    TooLong(usize),
    /// The value is not a valid identifier of the named `kind` (for example `"DOI"`).
    Invalid {
        /// The kind of identifier the value was checked as.
        kind: &'static str,
        /// The value as it was given, trimmed.
        value: String,
    },
}

impl fmt::Display for IdentifierError {
    /// A one-line message a user can read in the start screen.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "No identifier was given."),
            Self::TooLong(count) => write!(
                f,
                "The identifier is {count} characters long; the limit is {MAX_IDENTIFIER_CHARS}."
            ),
            Self::Invalid { kind, value } => write!(f, "\"{value}\" is not a valid {kind}."),
        }
    }
}

impl std::error::Error for IdentifierError {}

/// Whether `text` is one or more ASCII digits and nothing else.
fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Refuse `text` if it is longer than `MAX_IDENTIFIER_CHARS` characters.
fn check_length(text: &str) -> Result<(), IdentifierError> {
    let count = text.chars().count();
    if count > MAX_IDENTIFIER_CHARS {
        Err(IdentifierError::TooLong(count))
    } else {
        Ok(())
    }
}

/// Canonical PMID, or a preprint accession (`PPR` + digits, upper-cased).
fn check_pmid(text: &str) -> Result<String, IdentifierError> {
    if let Some(pmid) = canonical_pmid(text) {
        return Ok(pmid);
    }
    let upper = text.to_ascii_uppercase();
    if upper.strip_prefix(PREPRINT_PREFIX).is_some_and(is_digits) {
        return Ok(upper);
    }
    Err(IdentifierError::Invalid {
        kind: "PMID",
        value: text.to_string(),
    })
}

/// Whether a normalised DOI has the shape `10.` + 4 to 9 digits + `/` + a
/// suffix, with no whitespace, control or invisible formatting character
/// anywhere. Checked by hand, not by a regex.
fn is_doi_shape(doi: &str) -> bool {
    let Some(rest) = doi.strip_prefix(DOI_NAMESPACE) else {
        return false;
    };
    let Some((registrant, suffix)) = rest.split_once('/') else {
        return false;
    };
    is_digits(registrant)
        && (DOI_REGISTRANT_MIN_DIGITS..=DOI_REGISTRANT_MAX_DIGITS).contains(&registrant.len())
        && !suffix.is_empty()
        && !doi
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || is_invisible_format(c))
}

/// Zero-width and bidirectional formatting characters (U+200B–U+200F,
/// U+202A–U+202E, U+2066–U+2069): invisible, so a DOI holding one would
/// display as a different DOI from the one stored and sent.
fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The canonical DOI (lower case, prefix stripped) if `text` is a DOI.
fn check_doi(text: &str) -> Result<String, IdentifierError> {
    let invalid = || IdentifierError::Invalid {
        kind: "DOI",
        value: text.to_string(),
    };
    let doi = normalize_doi(Some(text)).ok_or_else(invalid)?;
    if is_doi_shape(&doi) {
        Ok(doi)
    } else {
        Err(invalid())
    }
}

/// The canonical PMC ID (`PMC` + ASCII digits, leading zeros stripped),
/// accepting a bare number or any case of the prefix. Digits are checked as
/// ASCII first, because bmlib's `\d` also matches other scripts.
fn check_pmc(text: &str) -> Result<String, IdentifierError> {
    let invalid = || IdentifierError::Invalid {
        kind: "PMC ID",
        value: text.to_string(),
    };
    let upper = text.to_ascii_uppercase();
    let digits = upper.strip_prefix(PMC_PREFIX).unwrap_or(&upper);
    if !is_digits(digits) {
        return Err(invalid());
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Err(invalid());
    }
    normalise_pmc_id(&format!("{PMC_PREFIX}{digits}")).map_err(|_| invalid())
}

/// Trim a field and check it, or return `None` when the field is blank or absent.
fn optional_field(
    raw: Option<&str>,
    check: fn(&str) -> Result<String, IdentifierError>,
) -> Result<Option<String>, IdentifierError> {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    check_length(text)?;
    check(text).map(Some)
}

/// Check each identifier field separately. Blank fields count as absent, and
/// at least one identifier must remain.
///
/// # Errors
///
/// [`IdentifierError::Empty`] if every field is blank or absent; otherwise the
/// first field that is too long or malformed.
pub fn validate_identifiers(
    pmid: Option<&str>,
    doi: Option<&str>,
    pmc_id: Option<&str>,
) -> Result<Identifiers, IdentifierError> {
    let ids = Identifiers {
        pmid: optional_field(pmid, check_pmid)?,
        doi: optional_field(doi, check_doi)?,
        pmc_id: optional_field(pmc_id, check_pmc)?,
    };
    if ids == Identifiers::default() {
        Err(IdentifierError::Empty)
    } else {
        Ok(ids)
    }
}

/// Classify one free-text identifier typed on the start screen. A `PMC`
/// prefix means a PMC ID, a `10.` and `/` mean a DOI, and anything else is
/// a PMID.
///
/// # Errors
///
/// [`IdentifierError::Empty`] for blank input, [`IdentifierError::TooLong`]
/// past the length limit, otherwise [`IdentifierError::Invalid`] naming the
/// kind it was classified as.
pub fn parse_identifier(input: &str) -> Result<Identifiers, IdentifierError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(IdentifierError::Empty);
    }
    check_length(text)?;
    let mut ids = Identifiers::default();
    if text.to_ascii_uppercase().starts_with(PMC_PREFIX) {
        ids.pmc_id = Some(check_pmc(text)?);
    } else if text.contains(DOI_NAMESPACE) && text.contains('/') {
        ids.doi = Some(check_doi(text)?);
    } else {
        ids.pmid = Some(check_pmid(text)?);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DOI of exactly `count` characters, for the length limit.
    fn doi_of_len(count: usize) -> String {
        let prefix = "10.1000/";
        format!("{prefix}{}", "a".repeat(count - prefix.len()))
    }

    /// A PMID with surrounding spaces and leading zeros is stored canonically.
    #[test]
    fn pmid_canonical() {
        let ids = validate_identifiers(Some(" 00012345 "), None, None).unwrap();
        assert_eq!(ids.pmid, Some("12345".to_string()));
    }

    /// A preprint accession is accepted as it is.
    #[test]
    fn pmid_preprint() {
        let ids = validate_identifiers(Some("PPR123456"), None, None).unwrap();
        assert_eq!(ids.pmid, Some("PPR123456".to_string()));
    }

    /// A PMID with a letter in it is refused, and the error names the PMID kind.
    #[test]
    fn pmid_invalid() {
        let err = validate_identifiers(Some("12a45"), None, None).unwrap_err();
        assert!(matches!(err, IdentifierError::Invalid { kind: "PMID", .. }));
    }

    /// A DOI URL is stripped and lower-cased to the bare form.
    #[test]
    fn doi_normalised() {
        let ids = validate_identifiers(None, Some("https://doi.org/10.1056/NEJMoa2034577"), None)
            .unwrap();
        assert_eq!(ids.doi, Some("10.1056/nejmoa2034577".to_string()));
    }

    /// Control characters (NUL, BEL, DEL) and invisible bidi or zero-width
    /// characters are refused inside a DOI.
    #[test]
    fn doi_control_characters_refused() {
        for bad in [
            "10.1000/a\u{0}b",
            "10.1000/x\u{7}",
            "10.1000/x\u{7f}",
            "10.1000/\u{202E}gpj",
            "10.1000/a\u{200B}b",
        ] {
            let err = validate_identifiers(None, Some(bad), None).unwrap_err();
            assert!(
                matches!(err, IdentifierError::Invalid { kind: "DOI", .. }),
                "{bad:?}"
            );
        }
    }

    /// A registrant code of one digit, and a DOI with a space, are both refused as DOIs.
    #[test]
    fn doi_invalid() {
        for bad in ["10.1/x", "10.1056/a b"] {
            let err = validate_identifiers(None, Some(bad), None).unwrap_err();
            assert!(
                matches!(err, IdentifierError::Invalid { kind: "DOI", .. }),
                "{bad} should be an invalid DOI"
            );
        }
    }

    /// A bare PMC number and a prefixed one give the same canonical value.
    #[test]
    fn pmc_bare_and_prefixed_agree() {
        let bare = validate_identifiers(None, None, Some("7654321")).unwrap();
        let prefixed = validate_identifiers(None, None, Some("PMC7654321")).unwrap();
        assert_eq!(bare, prefixed);
        assert_eq!(bare.pmc_id, Some("PMC7654321".to_string()));
    }

    /// Blank fields are absent, so all-blank input is `Empty`.
    #[test]
    fn all_blank_is_empty() {
        let err = validate_identifiers(Some(""), Some("  "), None).unwrap_err();
        assert_eq!(err, IdentifierError::Empty);
    }

    /// A 257-character DOI is refused by length, and the error reports the count.
    #[test]
    fn too_long() {
        let doi = doi_of_len(MAX_IDENTIFIER_CHARS + 1);
        let err = validate_identifiers(None, Some(&doi), None).unwrap_err();
        assert_eq!(err, IdentifierError::TooLong(MAX_IDENTIFIER_CHARS + 1));
    }

    /// Each kind of free-text input lands in its own field.
    #[test]
    fn parse_classifies() {
        assert_eq!(
            parse_identifier("pmc7654321").unwrap().pmc_id,
            Some("PMC7654321".to_string())
        );
        assert_eq!(
            parse_identifier("doi:10.1000/xyz").unwrap().doi,
            Some("10.1000/xyz".to_string())
        );
        assert_eq!(
            parse_identifier("34567").unwrap().pmid,
            Some("34567".to_string())
        );
    }

    /// Blank free text is `Empty`, not an invalid PMID.
    #[test]
    fn parse_blank_is_empty() {
        assert_eq!(parse_identifier("   ").unwrap_err(), IdentifierError::Empty);
    }

    /// PMC digits from other scripts are refused, though bmlib's `\\d` would accept them.
    #[test]
    fn pmc_non_ascii_digits_refused() {
        for bad in [
            "\u{0663}\u{0664}\u{0665}",
            "PMC\u{0663}\u{0664}\u{0665}",
            "\u{FF13}\u{FF14}\u{FF15}",
            "PMC\u{FF13}\u{FF14}\u{FF15}",
        ] {
            let err = validate_identifiers(None, None, Some(bad)).unwrap_err();
            assert!(
                matches!(err, IdentifierError::Invalid { kind: "PMC ID", .. }),
                "{bad} should be an invalid PMC ID"
            );
            // A bare number is classified as a PMID by `parse_identifier`, so only the prefixed form goes through it.
            if bad.starts_with("PMC") {
                assert!(matches!(
                    parse_identifier(bad).unwrap_err(),
                    IdentifierError::Invalid { kind: "PMC ID", .. }
                ));
            }
        }
    }

    /// PMC IDs lose leading zeros, whatever the prefix; `PMC0` and `PMC000` are refused.
    #[test]
    fn pmc_canonical_form() {
        for good in ["PMC007", "007", "pmc7", "PMC7"] {
            assert_eq!(
                validate_identifiers(None, None, Some(good)).unwrap().pmc_id,
                Some("PMC7".to_string()),
                "{good}"
            );
        }
        for bad in ["PMC0", "PMC000", "0"] {
            assert!(
                matches!(
                    validate_identifiers(None, None, Some(bad)).unwrap_err(),
                    IdentifierError::Invalid { kind: "PMC ID", .. }
                ),
                "{bad}"
            );
        }
    }

    /// A preprint accession is stored upper-case whatever case it arrived in.
    #[test]
    fn preprint_case_normalised() {
        let ids = validate_identifiers(Some("ppr123"), None, None).unwrap();
        assert_eq!(ids.pmid, Some("PPR123".to_string()));
    }

    /// Free text longer than the limit is refused by `parse_identifier` too.
    #[test]
    fn parse_too_long() {
        let text = "1".repeat(MAX_IDENTIFIER_CHARS + 1);
        assert_eq!(
            parse_identifier(&text).unwrap_err(),
            IdentifierError::TooLong(MAX_IDENTIFIER_CHARS + 1)
        );
    }

    /// A DOI with nothing after the slash, or a 10-digit registrant, is refused.
    #[test]
    fn doi_empty_suffix_and_long_registrant_refused() {
        for bad in ["10.1000/", "10.1000000000/x"] {
            assert!(
                matches!(
                    validate_identifiers(None, Some(bad), None).unwrap_err(),
                    IdentifierError::Invalid { kind: "DOI", .. }
                ),
                "{bad}"
            );
        }
    }
}
