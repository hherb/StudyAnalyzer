//! The error the frontend sees: a stable `kind` to branch on and a message to show.

use bmlib::llm::{ChatError, ContextWindowError, ListModelsError};
use bmlib::quality::PicoError;
use serde::{Serialize, Serializer};
use studyanalyzer_core::embedder::EmbedderError;
use studyanalyzer_core::intake::IntakeError;
use studyanalyzer_core::interrogate::AskError;
use studyanalyzer_core::llm_settings::SettingsError;
use studyanalyzer_core::open::OpenError;
use studyanalyzer_core::passages::PassageError;
use studyanalyzer_core::pico::PicoRunError;
use studyanalyzer_core::store::StoreError;

/// HTTP statuses with which a provider refuses the API key: unauthorised, forbidden.
const REJECTED_KEY_STATUSES: [u16; 2] = [401, 403];

/// The HTTP status with which a provider says it has no such model.
const NO_SUCH_MODEL_STATUS: u16 = 404;

/// A failure's category, the one thing the frontend branches on. It
/// serialises as its [`ErrorKind::as_str`] name. A new variant also goes into
/// `EVERY_KIND` in `contract.rs` (a match beside it stops compiling, as the
/// reminder) and into `ERROR_KINDS` in `src/lib/api.ts`; the contract test
/// then checks those two lists agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Asking needs a model and none is chosen in Settings.
    NeedsModel,
    /// The model's provider needs an API key and none is stored or set, or it
    /// refused the one given.
    NeedsApiKey,
    /// Answering needs the search model, which is not downloaded.
    NeedsEmbedder,
    /// The paper has no full text, or one neither checked nor confirmed whole.
    NotFullText,
    /// The model or its provider failed; the message names the provider.
    Llm,
    /// The user stopped the request.
    Cancelled,
    /// The database failed.
    Storage,
    /// The app failed in a way the user cannot fix.
    Internal,
}

impl ErrorKind {
    /// The kind's name on the wire and in the log (`invalid_input`, …), so
    /// Rust's log lines and the frontend's name a kind the same way.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::TooLarge => "too_large",
            ErrorKind::UnsupportedFormat => "unsupported_format",
            ErrorKind::Undecodable => "undecodable",
            ErrorKind::NeedsEmail => "needs_email",
            ErrorKind::NeedsModel => "needs_model",
            ErrorKind::NeedsApiKey => "needs_api_key",
            ErrorKind::NeedsEmbedder => "needs_embedder",
            ErrorKind::NotFullText => "not_full_text",
            ErrorKind::Llm => "llm",
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::Storage => "storage",
            ErrorKind::Internal => "internal",
        }
    }
}

impl Serialize for ErrorKind {
    /// Serialises as [`ErrorKind::as_str`].
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
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
        let name = kind.as_str();
        match kind {
            ErrorKind::Storage | ErrorKind::Internal | ErrorKind::Llm => {
                log::error!("command failed ({name}): {message}")
            }
            ErrorKind::Cancelled => log::info!("command cancelled: {message}"),
            ErrorKind::InvalidInput
            | ErrorKind::TooLarge
            | ErrorKind::UnsupportedFormat
            | ErrorKind::Undecodable
            | ErrorKind::NeedsEmail
            | ErrorKind::NeedsModel
            | ErrorKind::NeedsApiKey
            | ErrorKind::NeedsEmbedder
            | ErrorKind::NotFullText => log::warn!("command refused ({name}): {message}"),
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

    /// `provider` needs an API key, which `env_var` could also supply.
    pub(crate) fn needs_api_key(provider: &str, env_var: &str) -> Self {
        Self::new(
            ErrorKind::NeedsApiKey,
            format!("{provider} needs an API key: save one in Settings, or set {env_var}."),
        )
    }

    /// `provider` answered HTTP `status`: a refused key (401, 403) is
    /// `needs_api_key`, no such model (404) `invalid_input`, each keeping the
    /// server's `message`; any other status is `None`, for the caller to map.
    fn from_refusal(status: u16, message: &str, provider: &str) -> Option<Self> {
        if REJECTED_KEY_STATUSES.contains(&status) {
            Some(Self::new(
                ErrorKind::NeedsApiKey,
                format!("{provider} refused the API key: check it in Settings. ({message})"),
            ))
        } else if status == NO_SUCH_MODEL_STATUS {
            Some(Self::invalid_input(format!(
                "{provider} has no such model: check the model's name in Settings. ({message})"
            )))
        } else {
            None
        }
    }

    /// A chat call to `provider` failed: a missing or rejected key (HTTP 401,
    /// 403) is `needs_api_key`, a model the provider lacks (HTTP 404)
    /// `invalid_input`, anything else `llm` naming the provider. Each message
    /// keeps the server's own words.
    pub fn from_chat(e: ChatError, provider: &str) -> Self {
        if let ChatError::Http { status, message } = &e {
            if let Some(c) = Self::from_refusal(*status, message, provider) {
                return c;
            }
        }
        match e {
            ChatError::MissingApiKey { provider, env_var } => {
                Self::needs_api_key(&provider, &env_var)
            }
            e => Self::new(
                ErrorKind::Llm,
                format!("The model could not answer ({provider}): {e}"),
            ),
        }
    }

    /// Reading `model`'s context window failed: a model the provider lacks is
    /// `invalid_input`, a missing or refused key (HTTP 401, 403, bmlib #664)
    /// `needs_api_key`, the rest `llm`. A `Transport` 404 is a wrong address
    /// to bmlib (a 404 naming the model is `NoSuchModel`), so it stays `llm`.
    pub fn from_window(e: ContextWindowError, model: &str) -> Self {
        if let ContextWindowError::Transport {
            status: Some(status),
            message,
        } = &e
        {
            if REJECTED_KEY_STATUSES.contains(status) {
                if let Some(c) = Self::from_refusal(*status, message, model) {
                    return c;
                }
            }
        }
        match e {
            ContextWindowError::NoSuchModel { provider, model } => Self::invalid_input(format!(
                "{provider} has no model {model:?}: check the model's name."
            )),
            ContextWindowError::MissingApiKey { provider, env_var } => {
                Self::needs_api_key(&provider, &env_var)
            }
            e => Self::new(
                ErrorKind::Llm,
                format!("Could not read the context window of {model}: {e}"),
            ),
        }
    }

    /// A failed listing of `provider`'s models: a missing or refused key
    /// (HTTP 401, 403) is `needs_api_key`, the rest `llm` naming the
    /// provider, with the server's own words.
    pub fn from_list_models(e: ListModelsError, provider: &str) -> Self {
        match e {
            ListModelsError::MissingApiKey { provider, env_var } => {
                Self::needs_api_key(&provider, &env_var)
            }
            ListModelsError::Transport {
                status: Some(status),
                message,
            } if REJECTED_KEY_STATUSES.contains(&status) => Self::new(
                ErrorKind::NeedsApiKey,
                format!("{provider} refused the API key: check it in Settings. ({message})"),
            ),
            e => Self::new(
                ErrorKind::Llm,
                format!("Could not list {provider}'s models: {e}"),
            ),
        }
    }

    /// Maps a refused question; `provider` is the asked model's, for a chat failure.
    pub fn from_ask(e: AskError, provider: &str) -> Self {
        if let AskError::Chat(chat) = e {
            return Self::from_chat(chat, provider);
        }
        let kind = match &e {
            AskError::NeedsModel => ErrorKind::NeedsModel,
            AskError::Embedder(EmbedderError::Cancelled) => ErrorKind::Cancelled,
            AskError::Embedder(_) => ErrorKind::Internal,
            AskError::Store(_) => ErrorKind::Storage,
            AskError::EmptyQuestion | AskError::TooLong { .. } => ErrorKind::InvalidInput,
            AskError::NoFullText | AskError::NotFullText => ErrorKind::NotFullText,
            AskError::NeedsEmbedder => ErrorKind::NeedsEmbedder,
            AskError::Cancelled | AskError::Passages(PassageError::Cancelled) => {
                ErrorKind::Cancelled
            }
            // `Chat` returned above; listed so a new variant must be placed.
            AskError::Passages(_) | AskError::Internal(_) | AskError::Chat(_) => {
                ErrorKind::Internal
            }
        };
        Self::new(kind, e.to_string())
    }
}

impl CommandError {
    /// A PICO run on `provider`'s model stored nothing: a missing or
    /// unconfirmed text is `not_full_text`, a window too small
    /// `invalid_input`, a stop `cancelled`, a refused key or no such model as
    /// for a chat ([`CommandError::from_chat`], bmlib #662), the model's
    /// other failures `llm` naming the provider, a store failure `storage`,
    /// and a defect here or in bmlib `internal`. A refusal while a long paper
    /// is condensed carries its status too (bmlib #616).
    pub fn from_pico(e: PicoRunError, provider: &str) -> Self {
        if let PicoRunError::Pico(PicoError::RequestFailed {
            status: Some(status),
            message,
            ..
        }) = &e
        {
            if let Some(c) = Self::from_refusal(*status, message, provider) {
                return c;
            }
        }
        let kind = match &e {
            PicoRunError::NoFullText | PicoRunError::NotFullText => ErrorKind::NotFullText,
            PicoRunError::TooLong { .. } => ErrorKind::InvalidInput,
            PicoRunError::Cancelled | PicoRunError::Pico(PicoError::Stopped(_)) => {
                ErrorKind::Cancelled
            }
            PicoRunError::Store(_) => ErrorKind::Storage,
            PicoRunError::Internal(_)
            | PicoRunError::Pico(
                PicoError::InvalidSimilarity(_)
                | PicoError::EmptyInput
                | PicoError::PreparedOverBudget { .. }
                | PicoError::PreparedUnderBudget { .. },
            ) => ErrorKind::Internal,
            // `PicoError` is non-exhaustive: a later variant is the model's.
            PicoRunError::Pico(_) => ErrorKind::Llm,
        };
        let message = match kind {
            ErrorKind::Llm => format!("The model could not extract PICO ({provider}): {e}"),
            _ => e.to_string(),
        };
        Self::new(kind, message)
    }
}

impl From<IntakeError> for CommandError {
    /// Maps each intake failure to a kind; the message is the error's own text.
    fn from(e: IntakeError) -> Self {
        let kind = match &e {
            IntakeError::Unsupported(_) => ErrorKind::UnsupportedFormat,
            IntakeError::TooLarge { .. } => ErrorKind::TooLarge,
            IntakeError::Binary | IntakeError::Undecodable => ErrorKind::Undecodable,
            IntakeError::NotPdf(_)
            | IntakeError::Jats(_)
            | IntakeError::Html(_)
            | IntakeError::Empty(_) => ErrorKind::InvalidInput,
            IntakeError::Pdf(p) if p.kind.is_the_files() => ErrorKind::InvalidInput,
            // Any other PDF fault (the engine's; a cancel or deadline that
            // reached here, since a lookup turns the user's cancel into
            // `OpenError::Cancelled` and uploads have no limit), a scratch
            // file, or bmlib refusing a text our own converter built, is
            // our fault. Agrees with `IntakeError::is_the_documents`.
            IntakeError::Pdf(_) | IntakeError::Canonical(_) | IntakeError::Io(_) => {
                ErrorKind::Internal
            }
        };
        Self::new(kind, e.to_string())
    }
}

impl From<StoreError> for CommandError {
    /// Every storage failure is [`ErrorKind::Storage`].
    fn from(e: StoreError) -> Self {
        Self::new(ErrorKind::Storage, e.to_string())
    }
}

impl From<SettingsError> for CommandError {
    /// A refused value is the user's to fix; the store and the keychain are not.
    fn from(e: SettingsError) -> Self {
        let kind = match &e {
            SettingsError::Invalid(_) => ErrorKind::InvalidInput,
            SettingsError::Store(_) => ErrorKind::Storage,
            SettingsError::Secret(_) => ErrorKind::Internal,
            SettingsError::KeyUnavailable { .. } => ErrorKind::NeedsApiKey,
        };
        Self::new(kind, e.to_string())
    }
}

impl From<EmbedderError> for CommandError {
    /// The user's cancel is `cancelled`; any other failure is the app's.
    fn from(e: EmbedderError) -> Self {
        let kind = match &e {
            EmbedderError::Cancelled => ErrorKind::Cancelled,
            EmbedderError::Download(_) | EmbedderError::Check(_) | EmbedderError::Load(_) => {
                ErrorKind::Internal
            }
        };
        Self::new(kind, e.to_string())
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
            | OpenError::WrongPaper(_)
            | OpenError::Mismatch { .. }
            | OpenError::HandoffMismatch { .. }) => {
                Self::new(ErrorKind::InvalidInput, other.to_string())
            }
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
    use bmlib::fulltext::canonical::HtmlError;
    use bmlib::fulltext::pdf_text::PdfError;
    use bmlib::llm::{ChatError, ContextWindowError, ListModelsError};
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
        assert_eq!(k(IntakeError::NotPdf("x".into())), ErrorKind::InvalidInput);
        assert_eq!(k(IntakeError::Binary), ErrorKind::Undecodable);
        assert_eq!(k(IntakeError::Undecodable), ErrorKind::Undecodable);
        assert_eq!(k(IntakeError::Jats("x".into())), ErrorKind::InvalidInput);
        assert_eq!(
            k(IntakeError::Pdf(PdfError::unreadable("x"))),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            k(IntakeError::Pdf(PdfError::encrypted())),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            k(IntakeError::Pdf(PdfError::engine("x"))),
            ErrorKind::Internal
        );
        // Not the user's cancel, which arrives as `OpenError::Cancelled`.
        assert_eq!(
            k(IntakeError::Pdf(PdfError::cancelled())),
            ErrorKind::Internal
        );
        assert_eq!(
            k(IntakeError::Pdf(PdfError::timed_out())),
            ErrorKind::Internal
        );
        assert_eq!(k(IntakeError::Empty(None)), ErrorKind::InvalidInput);
        let timeout = HtmlError::TimedOut {
            parsed: 1,
            length: 2,
        };
        assert_eq!(k(IntakeError::Html(timeout)), ErrorKind::InvalidInput);
        assert_eq!(k(IntakeError::Canonical("x".into())), ErrorKind::Internal);
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
        let mismatch = CommandError::from(OpenError::Mismatch {
            paper: "\u{201c}A\u{201d}".into(),
            why: "its PMID is 2, but this paper's PMID is 1".into(),
            notices: vec!["Invalid PMC ID dropped.".into()],
        });
        assert_eq!(mismatch.kind, ErrorKind::InvalidInput);
        assert!(
            mismatch.message.contains("its PMID is 2")
                && mismatch
                    .message
                    .ends_with("in Settings. Invalid PMC ID dropped."),
            "{}",
            mismatch.message
        );
        let handed = CommandError::from(OpenError::HandoffMismatch {
            sender: "MFC".into(),
            why: "its PMID is 2, but the PMID sent with it is 1".into(),
            notices: vec![],
        });
        assert_eq!(handed.kind, ErrorKind::InvalidInput);
        assert!(
            handed.message.contains("MFC handed over"),
            "{}",
            handed.message
        );
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

    /// One assertion per `AskError` variant; a chat failure names the provider.
    #[test]
    fn each_ask_error_maps_to_a_kind() {
        use studyanalyzer_core::interrogate::AskError;
        use studyanalyzer_core::passages::PassageError;
        let k = |e: AskError| CommandError::from_ask(e, "anthropic").kind;
        assert_eq!(k(AskError::NeedsModel), ErrorKind::NeedsModel);
        assert_eq!(k(AskError::EmptyQuestion), ErrorKind::InvalidInput);
        assert_eq!(k(AskError::NoFullText), ErrorKind::NotFullText);
        assert_eq!(k(AskError::NotFullText), ErrorKind::NotFullText);
        assert_eq!(k(AskError::NeedsEmbedder), ErrorKind::NeedsEmbedder);
        let long = |has_history| AskError::TooLong {
            window: 8192,
            has_history,
        };
        assert_eq!(k(long(true)), ErrorKind::InvalidInput);
        assert_eq!(k(long(false)), ErrorKind::InvalidInput);
        assert_eq!(k(AskError::Cancelled), ErrorKind::Cancelled);
        let chat = CommandError::from_ask(
            AskError::Chat(ChatError::Transport("refused".into())),
            "anthropic",
        );
        assert_eq!(chat.kind, ErrorKind::Llm);
        assert!(
            chat.message.contains("anthropic") && chat.message.contains("refused"),
            "{}",
            chat.message
        );
        assert_eq!(
            k(AskError::Chat(ChatError::MissingApiKey {
                provider: "anthropic".into(),
                env_var: "ANTHROPIC_API_KEY".into(),
            })),
            ErrorKind::NeedsApiKey
        );
        assert_eq!(
            k(AskError::Passages(PassageError::Cancelled)),
            ErrorKind::Cancelled
        );
        assert_eq!(
            k(AskError::Store(StoreError::Corrupt("x".into()))),
            ErrorKind::Storage
        );
        assert_eq!(k(AskError::Internal("x".into())), ErrorKind::Internal);
        assert_eq!(
            k(AskError::Embedder(EmbedderError::Load("x".into()))),
            ErrorKind::Internal
        );
    }

    /// One assertion per `PicoRunError` variant and, under `Pico`, per
    /// `PicoError` variant; the model's failure names the provider.
    #[test]
    fn each_pico_error_maps_to_a_kind() {
        let k = |e: PicoRunError| CommandError::from_pico(e, "anthropic").kind;
        assert_eq!(k(PicoRunError::NoFullText), ErrorKind::NotFullText);
        assert_eq!(k(PicoRunError::NotFullText), ErrorKind::NotFullText);
        assert_eq!(
            k(PicoRunError::TooLong { window: None }),
            ErrorKind::InvalidInput
        );
        assert_eq!(k(PicoRunError::Cancelled), ErrorKind::Cancelled);
        assert_eq!(
            k(PicoRunError::Store(StoreError::Corrupt("x".into()))),
            ErrorKind::Storage
        );
        assert_eq!(k(PicoRunError::Internal("x".into())), ErrorKind::Internal);
        let p = |e: PicoError| k(PicoRunError::Pico(e));
        assert_eq!(p(PicoError::Stopped(None)), ErrorKind::Cancelled);
        assert_eq!(p(PicoError::EmptyInput), ErrorKind::Internal);
        assert_eq!(p(PicoError::InvalidSimilarity(2.0)), ErrorKind::Internal);
        let budget = PicoError::PreparedOverBudget {
            chars: 2,
            budget: 1,
        };
        assert_eq!(p(budget), ErrorKind::Internal);
        let budget = PicoError::PreparedUnderBudget {
            chars: 1,
            budget: 2,
        };
        assert_eq!(p(budget), ErrorKind::Internal);
        for e in [
            PicoError::RequestFailed {
                message: "refused".into(),
                retryable: false,
                status: None,
            },
            PicoError::RequestFailed {
                message: "refused".into(),
                retryable: true,
                status: Some(503),
            },
            PicoError::Truncated("refused".into()),
            PicoError::UnusableReply("refused".into()),
            PicoError::CondensationFailed("refused".into()),
        ] {
            let c = CommandError::from_pico(PicoRunError::Pico(e), "anthropic");
            assert_eq!(c.kind, ErrorKind::Llm);
            assert!(c.message.contains("anthropic"), "{}", c.message);
        }
        // A refusal reads as a chat's does: the key or the model's name to fix.
        let http = |status| {
            CommandError::from_pico(
                PicoRunError::Pico(PicoError::RequestFailed {
                    message: format!("HTTP {status}: says the server"),
                    retryable: false,
                    status: Some(status),
                }),
                "mistral",
            )
        };
        for (status, kind) in [
            (401, ErrorKind::NeedsApiKey),
            (403, ErrorKind::NeedsApiKey),
            (404, ErrorKind::InvalidInput),
        ] {
            let c = http(status);
            assert_eq!(c.kind, kind, "{status}");
            assert!(
                c.message.contains("mistral") && c.message.contains("says the server"),
                "{}",
                c.message
            );
        }
    }

    /// One assertion per `EmbedderError` variant: only the cancel is the user's.
    #[test]
    fn each_embedder_error_maps_to_a_kind() {
        use bmlib::embed::ModelError;
        let k = |e: EmbedderError| CommandError::from(e).kind;
        assert_eq!(k(EmbedderError::Cancelled), ErrorKind::Cancelled);
        assert_eq!(
            k(EmbedderError::Download(ModelError::Transport("x".into()))),
            ErrorKind::Internal
        );
        let io = ModelError::Io(std::io::Error::other("x"));
        assert_eq!(k(EmbedderError::Check(io)), ErrorKind::Internal);
        assert_eq!(k(EmbedderError::Load("x".into())), ErrorKind::Internal);
    }

    /// One assertion per `ChatError` variant: a missing key is `needs_api_key`
    /// naming its variable, a refused key (HTTP 401, 403) `needs_api_key` and
    /// no such model (HTTP 404) `invalid_input`, both keeping the server's
    /// words; every other failure, other statuses too, is `llm` naming the provider.
    #[test]
    fn each_chat_error_maps_to_a_kind() {
        let missing = CommandError::from_chat(
            ChatError::MissingApiKey {
                provider: "anthropic".into(),
                env_var: "ANTHROPIC_API_KEY".into(),
            },
            "anthropic",
        );
        assert_eq!(missing.kind, ErrorKind::NeedsApiKey);
        assert!(
            missing.message.contains("ANTHROPIC_API_KEY"),
            "{}",
            missing.message
        );
        let http = |status| {
            CommandError::from_chat(
                ChatError::Http {
                    status,
                    message: format!("HTTP {status}: says the server"),
                },
                "mistral",
            )
        };
        for status in [401, 403] {
            let rejected = http(status);
            assert_eq!(rejected.kind, ErrorKind::NeedsApiKey, "{status}");
            assert!(
                rejected.message.contains("mistral")
                    && rejected.message.contains("says the server"),
                "{}",
                rejected.message
            );
        }
        let no_model = http(404);
        assert_eq!(no_model.kind, ErrorKind::InvalidInput);
        assert!(
            no_model.message.contains("says the server"),
            "{}",
            no_model.message
        );
        for e in [
            ChatError::UnknownProvider("x".into()),
            ChatError::ToolsUnsupported("x".into()),
            ChatError::Transport("x".into()),
            ChatError::Http {
                status: 503,
                message: "x".into(),
            },
            ChatError::Http {
                status: 400,
                message: "x".into(),
            },
            ChatError::Malformed("x".into()),
            ChatError::Stream("x".into()),
            ChatError::StreamingToolsUnsupported,
        ] {
            let c = CommandError::from_chat(e, "mistral");
            assert_eq!(c.kind, ErrorKind::Llm);
            assert!(c.message.contains("mistral"), "{}", c.message);
        }
    }

    /// A model the provider lacks is `invalid_input` naming it; a missing or
    /// refused key is `needs_api_key`; the rest is `llm`.
    #[test]
    fn each_window_error_maps_to_a_kind() {
        let k = |e| CommandError::from_window(e, "ollama:nope");
        let none = k(ContextWindowError::NoSuchModel {
            provider: "ollama".into(),
            model: "nope".into(),
        });
        assert_eq!(none.kind, ErrorKind::InvalidInput);
        assert!(none.message.contains("nope"), "{}", none.message);
        let missing = k(ContextWindowError::MissingApiKey {
            provider: "anthropic".into(),
            env_var: "ANTHROPIC_API_KEY".into(),
        });
        assert_eq!(missing.kind, ErrorKind::NeedsApiKey);
        for status in REJECTED_KEY_STATUSES {
            let refused = k(ContextWindowError::Transport {
                message: "HTTP 401: bad key".into(),
                status: Some(status),
            });
            assert_eq!(refused.kind, ErrorKind::NeedsApiKey, "{status}");
            assert!(refused.message.contains("bad key"), "{}", refused.message);
        }
        for e in [
            ContextWindowError::UnknownProvider("x".into()),
            ContextWindowError::Transport {
                message: "x".into(),
                status: None,
            },
            // A 404 naming no model is a wrong address, not a missing model.
            ContextWindowError::Transport {
                message: "x".into(),
                status: Some(404),
            },
            ContextWindowError::Transport {
                message: "x".into(),
                status: Some(503),
            },
            ContextWindowError::Malformed("x".into()),
        ] {
            let c = k(e);
            assert_eq!(c.kind, ErrorKind::Llm);
            assert!(c.message.contains("ollama:nope"), "{}", c.message);
        }
    }

    /// One assertion per `ListModelsError` variant, and each refused status.
    #[test]
    fn each_list_models_error_maps_to_a_kind() {
        let k = |e| CommandError::from_list_models(e, "anthropic");
        let missing = k(ListModelsError::MissingApiKey {
            provider: "anthropic".into(),
            env_var: "ANTHROPIC_API_KEY".into(),
        });
        assert_eq!(missing.kind, ErrorKind::NeedsApiKey);
        for status in REJECTED_KEY_STATUSES {
            let refused = k(ListModelsError::Transport {
                message: "HTTP 401: bad key".into(),
                status: Some(status),
            });
            assert_eq!(refused.kind, ErrorKind::NeedsApiKey, "{status}");
            assert!(refused.message.contains("bad key"), "{}", refused.message);
        }
        for e in [
            ListModelsError::UnknownProvider("x".into()),
            ListModelsError::Transport {
                message: "x".into(),
                status: None,
            },
            ListModelsError::Transport {
                message: "x".into(),
                status: Some(404),
            },
            ListModelsError::Malformed("x".into()),
        ] {
            let c = k(e);
            assert_eq!(c.kind, ErrorKind::Llm);
            assert!(c.message.contains("anthropic"), "{}", c.message);
        }
    }

    /// One assertion per `SettingsError` variant.
    #[test]
    fn each_settings_error_maps_to_a_kind() {
        use studyanalyzer_core::llm_settings::{SecretError, SettingsError};
        assert_eq!(
            CommandError::from(SettingsError::Invalid("x".into())).kind,
            ErrorKind::InvalidInput
        );
        assert_eq!(
            CommandError::from(SettingsError::Store(StoreError::Corrupt("x".into()))).kind,
            ErrorKind::Storage
        );
        assert_eq!(
            CommandError::from(SettingsError::Secret(SecretError("locked".into()))).kind,
            ErrorKind::Internal
        );
        let unavailable = CommandError::from(SettingsError::KeyUnavailable {
            provider: "anthropic".into(),
            env_var: "ANTHROPIC_API_KEY".into(),
            reason: "locked".into(),
        });
        assert_eq!(unavailable.kind, ErrorKind::NeedsApiKey);
        assert!(
            unavailable.message.contains("keychain"),
            "{}",
            unavailable.message
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
