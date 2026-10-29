//! Every limit, default and name the core uses lives here (no magic numbers).

/// The product name shown in window titles and messages.
pub const APP_NAME: &str = "StudyAnalyzer";

/// The longest free-text or stored identifier accepted, in characters.
/// The check runs after surrounding whitespace is trimmed, so padding does not
/// count. Anything longer is refused before it is looked at, so a hostile
/// hand-off file cannot push an oversized string through the validators.
pub const MAX_IDENTIFIER_CHARS: usize = 256;

/// The prefix of a PMC ID, the PubMed Central archive's identifier.
pub const PMC_PREFIX: &str = "PMC";

/// The prefix of a preprint accession (PubMed's `PPR` numbers), followed by digits.
pub const PREPRINT_PREFIX: &str = "PPR";

/// The leading part of every DOI, the Crossref/DataCite namespace.
pub const DOI_NAMESPACE: &str = "10.";

/// The fewest digits a DOI registrant code may have.
pub const DOI_REGISTRANT_MIN_DIGITS: usize = 4;

/// The most digits a DOI registrant code may have.
pub const DOI_REGISTRANT_MAX_DIGITS: usize = 9;

#[cfg(test)]
mod tests {
    use super::*;

    /// The product name is the one the bundle and the UI use.
    #[test]
    fn app_name_is_studyanalyzer() {
        assert_eq!(APP_NAME, "StudyAnalyzer");
    }
}
