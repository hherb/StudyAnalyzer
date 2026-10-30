//! The error the frontend sees: a stable `kind` to branch on and a message to show.

use serde::Serialize;
use studyanalyzer_core::intake::IntakeError;
use studyanalyzer_core::open::OpenError;
use studyanalyzer_core::store::StoreError;

/// A failure's category, the one thing the frontend branches on. It
/// serialises as snake_case (`invalid_input`, …); `src/lib/api.ts` mirrors
/// the set by hand, and the contract fixture checks the two agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The user's input is unusable (they can fix it).
    InvalidInput,
    /// The file or text is over its size cap.
    TooLarge,
    /// The file is not a format the app reads.
    UnsupportedFormat,
    /// The bytes could not be decoded as text.
    Undecodable,
    /// Identifier retrieval needs a contact email and none is stored.
    NeedsEmail,
    /// The user stopped the request.
    Cancelled,
    /// The database failed.
    Storage,
    /// The app failed in a way the user cannot fix.
    Internal,
}

/// What a command returns on failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    /// A stable machine-readable category.
    pub kind: ErrorKind,
    /// A message the user can read.
    pub message: String,
}

impl CommandError {
    /// Builds the error and logs it: `warn` for the user's own input
    /// (they can fix it), `error` for anything the app is at fault for, `info`
    /// for a cancel (the user's choice).
    pub fn new(kind: ErrorKind, message: String) -> Self {
        match kind {
            ErrorKind::Storage | ErrorKind::Internal => {
                log::error!("command failed ({kind:?}): {message}")
            }
            ErrorKind::Cancelled => log::info!("command cancelled: {message}"),
            ErrorKind::InvalidInput
            | ErrorKind::TooLarge
            | ErrorKind::UnsupportedFormat
            | ErrorKind::Undecodable
            | ErrorKind::NeedsEmail => log::warn!("command refused ({kind:?}): {message}"),
        }
        CommandError { kind, message }
    }

    /// The user's input is unusable.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message.into())
    }

    /// The app failed in a way the user cannot fix.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message.into())
    }

    /// Identifier retrieval needs a contact email and none is stored.
    pub fn needs_email() -> Self {
        Self::new(
            ErrorKind::NeedsEmail,
            "Enter a contact email first: the free full-text services ask for one.".into(),
        )
    }
}

impl From<IntakeError> for CommandError {
    /// Maps each intake failure to a kind; the message is the error's own text.
    fn from(e: IntakeError) -> Self {
        let kind = match &e {
            IntakeError::Unsupported(_) => ErrorKind::UnsupportedFormat,
            IntakeError::TooLarge { .. } => ErrorKind::TooLarge,
            IntakeError::Binary | IntakeError::Undecodable => ErrorKind::Undecodable,
            IntakeError::Jats(_)
            | IntakeError::Pdf(_)
            | IntakeError::Empty
            | IntakeError::Canonical(_) => ErrorKind::InvalidInput,
            IntakeError::Io(_) => ErrorKind::Internal,
        };
        Self::new(kind, e.to_string())
    }
}

impl From<StoreError> for CommandError {
    /// Every storage failure is `storage`.
    fn from(e: StoreError) -> Self {
        Self::new(ErrorKind::Storage, e.to_string())
    }
}

impl From<OpenError> for CommandError {
    /// Delegates to the inner error's mapping where there is one.
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::Intake(e) => e.into(),
            OpenError::Store(e) => e.into(),
            other @ (OpenError::Identifier(_)
            | OpenError::UnknownPaper(_)
            | OpenError::WrongPaper(_)) => Self::new(ErrorKind::InvalidInput, other.to_string()),
            OpenError::Cancelled => {
                Self::new(ErrorKind::Cancelled, OpenError::Cancelled.to_string())
            }
        }
    }
}

/// Checks a contact email has exactly one `@` with text on both sides and
/// returns it trimmed. Deliberately loose: the services only need a contact.
pub fn check_contact_email(email: &str) -> Result<String, CommandError> {
    let email = email.trim();
    let mut parts = email.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) if !local.is_empty() && !domain.is_empty() => {
            Ok(email.to_string())
        }
        _ => Err(CommandError::invalid_input(
            "That does not look like an email address (expected name@domain).",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studyanalyzer_core::identifiers::parse_identifier;
    use studyanalyzer_core::intake::SourceKind;

    /// One assertion per error variant.
    #[test]
    fn each_open_error_maps_to_a_kind() {
        let k = |e: IntakeError| CommandError::from(OpenError::Intake(e)).kind;
        assert_eq!(
            k(IntakeError::Unsupported("x".into())),
            ErrorKind::UnsupportedFormat
        );
        assert_eq!(k(IntakeError::Binary), ErrorKind::Undecodable);
        assert_eq!(k(IntakeError::Undecodable), ErrorKind::Undecodable);
        assert_eq!(k(IntakeError::Jats("x".into())), ErrorKind::InvalidInput);
        assert_eq!(k(IntakeError::Pdf("x".into())), ErrorKind::InvalidInput);
        assert_eq!(k(IntakeError::Empty), ErrorKind::InvalidInput);
        assert_eq!(
            k(IntakeError::Canonical("x".into())),
            ErrorKind::InvalidInput
        );
        assert_eq!(k(IntakeError::Io("x".into())), ErrorKind::Internal);
        let big = CommandError::from(OpenError::Intake(IntakeError::TooLarge {
            kind: SourceKind::Pdf,
            len: 200 * 1024 * 1024,
            max: 100 * 1024 * 1024,
        }));
        assert_eq!(big.kind, ErrorKind::TooLarge);
        assert!(big.message.contains("100.0 MB"), "{}", big.message);
        let id = parse_identifier("not an id").unwrap_err();
        assert_eq!(
            CommandError::from(OpenError::Identifier(id)).kind,
            ErrorKind::InvalidInput
        );
        let wrong = CommandError::from(OpenError::WrongPaper("its pmid is 1".into()));
        assert_eq!(wrong.kind, ErrorKind::InvalidInput);
        assert!(wrong.message.contains("its pmid is 1"));
        let unknown = CommandError::from(OpenError::UnknownPaper("p1".into()));
        assert_eq!(unknown.kind, ErrorKind::InvalidInput);
        assert!(unknown.message.contains("p1"));
        let s = StoreError::NotFound("x".into());
        assert_eq!(
            CommandError::from(OpenError::Store(s)).kind,
            ErrorKind::Storage
        );
        assert_eq!(
            CommandError::from(StoreError::Corrupt("x".into())).kind,
            ErrorKind::Storage
        );
        assert_eq!(CommandError::needs_email().kind, ErrorKind::NeedsEmail);
        assert_eq!(
            CommandError::from(OpenError::Cancelled).kind,
            ErrorKind::Cancelled
        );
    }

    /// `a@b` passes; no `@`, an empty side or two `@` are refused.
    #[test]
    fn contact_email_checked() {
        assert_eq!(check_contact_email(" a@b ").unwrap(), "a@b");
        for bad in ["ab", "@b", "a@", "a@@b", "a@b@c", ""] {
            assert_eq!(
                check_contact_email(bad).unwrap_err().kind,
                ErrorKind::InvalidInput
            );
        }
    }
}
