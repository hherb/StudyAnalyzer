//! Every limit, default and name the core uses lives here (no magic numbers).

/// The product name shown in window titles and messages.
pub const APP_NAME: &str = "StudyAnalyzer";

#[cfg(test)]
mod tests {
    use super::*;

    /// The product name is the one the bundle and the UI use.
    #[test]
    fn app_name_is_studyanalyzer() {
        assert_eq!(APP_NAME, "StudyAnalyzer");
    }
}
