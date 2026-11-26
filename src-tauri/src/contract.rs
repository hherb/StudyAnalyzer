//! The JSON contract with the frontend: every shape `src/lib/api.ts` mirrors by
//! hand, pinned in a fixture that `src/lib/contract.test.ts` reads too.

use bmlib::fulltext::canonical::TextSpan;
use bmlib::fulltext::canonical::{
    assess_full_text, canonical_from_markdown, BlockKind, CanonicalText,
};
use serde_json::{json, Value};
use studyanalyzer_core::appraisal_store::{
    confirm, edit, save, start_appraisal, AppraisalEdit, ConfirmInput, ConfirmedOutcomeInput,
};
use studyanalyzer_core::appraisal_view::{
    appraisal_view, AppraisalEvent, AppraisalView, DamagedAppraisal,
};
use studyanalyzer_core::config::EMBEDDING_MODEL;
use studyanalyzer_core::conversations::{
    create_conversation, save_turn, ConversationSummary, Mode, NewQuote, NewTurn, TurnStatus,
};
use studyanalyzer_core::evidence::EvidenceEvent;
use studyanalyzer_core::evidence_store::{
    create_run as create_evidence_run, fail_step, pending_steps, save_design, save_domain,
    save_escalation, save_outcomes, EvidenceStep, StepFailure, PAIRED_STEP_FAILED,
};
use studyanalyzer_core::evidence_view::{
    evidence_run_view, evidence_view, EvidenceRunView, EvidenceView,
};
use studyanalyzer_core::handoff_store::HandoffSummary;
use studyanalyzer_core::identifiers::Identifiers;
use studyanalyzer_core::intake::{Decoded, SourceKind};
use studyanalyzer_core::interrogate::AnswerEvent;
use studyanalyzer_core::library_clear::{ListedFile, ListedPaper, Listing};
use studyanalyzer_core::library_setting::{Changed, LibraryStatus, PaperProblem, WaitingSource};
use studyanalyzer_core::llm_settings::{KeySource, LlmSettings, ProviderInfo};
use studyanalyzer_core::open::OpenOutcome;
use studyanalyzer_core::pico::PicoEvent;
use studyanalyzer_core::pico_store::{save_run, NewPicoRun, PicoCheck, PicoOutcome};
use studyanalyzer_core::prisma::PrismaEvent;
use studyanalyzer_core::prisma_store::{
    create_run as create_prisma_run, fail_part, pending_parts, save_not_suitable, save_part,
    PrismaCheck,
};
use studyanalyzer_core::prisma_view::{prisma_run_view, prisma_view, PrismaRunView, PrismaView};
use studyanalyzer_core::store::{
    create_paper, open_memory_store, save_canonical, save_source, NewSource, PaperSummary,
    PlaceCounts, SourcePlace, StoredCanonical,
};
use studyanalyzer_core::view::{
    conversation_view, paper_view, pico_run_view, pico_view, ConversationView, InterrogatorView,
    PicoRunView, PicoView,
};

use crate::commands::{DownloadProgress, EmbedderStatus};
use crate::error::{CommandError, ErrorKind};
use crate::grants::GrantedFile;
use crate::handoff::{FILES_DROPPED_EVENT, HANDOFF_EVENT};
use crate::{LibrarySettled, LIBRARY_SETTLED_EVENT};

/// The fixture both sides read.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/lib/fixtures/ipc_contract.json"
);

/// A paper with astral characters (two UTF-16 units each), a table and a
/// heading the full-text check does not recognise, so the banner has
/// something to name. `src/lib/contract.test.ts` asserts on its layout (one
/// section: paragraph, two table rows, paragraph) and its banner (both
/// headings missing, 8 words), so change the two together.
const PAPER: &str =
    "# Trial 😀\n\n## Méthods 𝛼\n\nBody 😀 text.\n\n| a | b😀 |\n|---|---|\n| 1 | 2 |\n\nAfter.\n";

/// A paper with no title and no heading, so the fixture pins the `null`s
/// (`title_span`, `heading`, and `decoded` for a file that needed none).
const BARE_PAPER: &str = "Just a body.\n";

/// Every error kind, kept by hand. [`every_kind_is_listed`] stops compiling
/// when `ErrorKind` gains one, to send its author here; add it here and to
/// `ERROR_KINDS` in `api.ts`.
const EVERY_KIND: [ErrorKind; 13] = [
    ErrorKind::InvalidInput,
    ErrorKind::TooLarge,
    ErrorKind::UnsupportedFormat,
    ErrorKind::Undecodable,
    ErrorKind::NeedsEmail,
    ErrorKind::NeedsModel,
    ErrorKind::NeedsApiKey,
    ErrorKind::NeedsEmbedder,
    ErrorKind::NotFullText,
    ErrorKind::Llm,
    ErrorKind::Cancelled,
    ErrorKind::Storage,
    ErrorKind::Internal,
];

/// Stops compiling when `ErrorKind` gains a variant. Add the variant here
/// and to [`EVERY_KIND`], or the fixture never shows the frontend the new kind.
fn every_kind_is_listed(kind: ErrorKind) {
    match kind {
        ErrorKind::InvalidInput
        | ErrorKind::TooLarge
        | ErrorKind::UnsupportedFormat
        | ErrorKind::Undecodable
        | ErrorKind::NeedsEmail
        | ErrorKind::NeedsModel
        | ErrorKind::NeedsApiKey
        | ErrorKind::NeedsEmbedder
        | ErrorKind::NotFullText
        | ErrorKind::Llm
        | ErrorKind::Cancelled
        | ErrorKind::Storage
        | ErrorKind::Internal => {}
    }
}

/// Fails to compile when a bmlib bump adds a `BlockKind`: add it to
/// `BlockView.kind` in `api.ts`, to `groupBlocks`, and to [`PAPER`].
fn every_block_kind_is_mirrored(kind: BlockKind) {
    match kind {
        BlockKind::Paragraph | BlockKind::TableRow => {}
    }
}

/// Every command `api.ts` invokes, with the argument keys it sends and each
/// one's JSON type. [`every_command_decodes_its_arguments`] sends these to the
/// real handlers; `contract.test.ts` checks the `api.ts` wrappers it calls send
/// exactly these. Both sides fail when a command is missing here: a new command
/// goes here, in `handler()`, in `api.ts` and in that test's call list together.
fn commands() -> Value {
    json!({
        "pick_file": {},
        "open_file": {"grant": "number"},
        "open_paste": {"text": "string"},
        "open_identifier": {"input": "string"},
        "cancel_lookup": {},
        "attach_file": {"paperId": "string", "grant": "number"},
        "attach_paste": {"paperId": "string", "text": "string"},
        "reopen_paper": {"paperId": "string"},
        "recent_papers": {},
        "paper_view": {"paperId": "string"},
        "original_pdf": {"paperId": "string"},
        "confirm_full_text": {"paperId": "string", "canonicalId": "number"},
        "library_status": {},
        "library_settle_listening": {},
        "choose_library": {"bring": "boolean"},
        "clear_library": {"bring": "boolean"},
        "move_into_library": {},
        "library_papers": {},
        "remove_library_paper": {"folder": "string"},
        "remove_library_file": {"folder": "string", "name": "string"},
        "get_contact_email": {},
        "set_contact_email": {"email": "string"},
        "get_keep_mismatched": {},
        "set_keep_mismatched": {"keep": "boolean"},
        "ask": {"paperId": "string", "question": "string", "onEvent": "channel"},
        "stop_answer": {},
        "new_conversation": {"paperId": "string"},
        "interrogator": {"paperId": "string"},
        "conversation": {"conversationId": "number"},
        "pico": {"paperId": "string"},
        "pico_run": {"runId": "number"},
        "run_pico": {"paperId": "string", "force": "boolean", "onEvent": "channel"},
        "stop_pico": {},
        "prisma": {"paperId": "string"},
        "prisma_run": {"runId": "number"},
        "run_prisma": {"paperId": "string", "force": "boolean", "continueRun": "number", "onEvent": "channel"},
        "stop_prisma": {},
        "evidence": {"paperId": "string"},
        "evidence_run": {"runId": "number"},
        "run_evidence": {"paperId": "string", "continueRun": "number", "onEvent": "channel"},
        "stop_evidence": {},
        "appraisal": {"appraisalId": "number", "version": "number"},
        "start_appraisal": {"paperId": "string"},
        "confirm_appraisal": {"appraisalId": "number", "input": "object"},
        "edit_appraisal": {"appraisalId": "number", "edit": "object"},
        "save_appraisal": {"appraisalId": "number"},
        "reveal_appraisal": {"appraisalId": "number"},
        "close_revision": {"appraisalId": "number"},
        "set_aside_appraisal": {"appraisalId": "number"},
        "run_appraisal": {"appraisalId": "number", "again": "boolean", "onEvent": "channel"},
        "paper_view_at": {"paperId": "string", "canonicalId": "number"},
        "llm_settings": {},
        "set_model": {"model": "string"},
        "set_context_tokens": {"tokens": "number"},
        "clear_context_tokens": {},
        "set_api_key": {"provider": "string", "key": "string"},
        "delete_api_key": {"provider": "string"},
        "model_window": {"model": "string"},
        "list_models": {"provider": "string"},
        "embedder_status": {},
        "download_embedder": {"onProgress": "channel"},
        "cancel_download": {},
        "pending_handoffs": {},
        "open_handoff": {"id": "number"},
        "discard_handoff": {"id": "number"},
    })
}

/// Each Rust return type a command declares (its `Ok` type), with the type
/// `api.ts` must give `invoke` for it. A `Response` arrives as raw bytes, not
/// JSON. A command returning a type not listed here fails [`returns`]: add it
/// with the type the frontend receives.
const RETURN_TYPES: [(&str, &str); 27] = [
    ("OpenOutcome", "OpenOutcome"),
    ("Vec<PaperSummary>", "PaperSummary[]"),
    ("PaperView", "PaperView"),
    ("Response", "ArrayBuffer"),
    ("()", "void"),
    ("Option<String>", "string | null"),
    ("bool", "boolean"),
    ("ConversationView", "ConversationView"),
    ("InterrogatorView", "InterrogatorView"),
    ("PicoView", "PicoView"),
    ("PicoRunView", "PicoRunView"),
    ("PrismaView", "PrismaView"),
    ("PrismaRunView", "PrismaRunView"),
    ("EvidenceView", "EvidenceView"),
    ("EvidenceRunView", "EvidenceRunView"),
    ("AppraisalView", "AppraisalView"),
    ("LlmSettings", "LlmSettings"),
    ("Option<u64>", "number | null"),
    ("Vec<String>", "string[]"),
    ("EmbedderStatus", "EmbedderStatus"),
    ("Option<GrantedFile>", "GrantedFile | null"),
    ("Vec<HandoffSummary>", "HandoffSummary[]"),
    ("LibraryStatus", "LibraryStatus"),
    ("Option<LibrarySettled>", "LibrarySettled | null"),
    ("Option<Changed>", "Changed | null"),
    ("Changed", "Changed"),
    ("Listing", "LibraryListing"),
];

/// The `Ok` type the handler `name` declares in `commands.rs`,
/// `evidence_commands.rs` or `appraisal_commands.rs`, read from its signature as written: `()` when it
/// returns nothing, the `T` of `Result<T, CommandError>` otherwise.
fn declared_return(src: &str, name: &str) -> String {
    let at = [format!("pub async fn {name}<"), format!("pub fn {name}<")]
        .iter()
        .find_map(|head| src.find(head.as_str()))
        .unwrap_or_else(|| panic!("no handler named {name} in the command files"));
    let signature = &src[at..at + src[at..].find('{').unwrap()];
    match signature.split_once("->") {
        None => "()".into(),
        Some((_, ret)) => ret
            .trim()
            .strip_prefix("Result<")
            .and_then(|r| r.strip_suffix(", CommandError>"))
            .unwrap_or_else(|| panic!("{name} returns {ret}, not Result<_, CommandError>"))
            .to_string(),
    }
}

/// Every registered command with the type `api.ts` must give its `invoke`,
/// derived from the handler's declared return type through [`RETURN_TYPES`],
/// so a handler that changes what it returns changes the fixture.
fn returns() -> Value {
    // The evidence weight's and the appraisal's commands live in files of their own.
    let src = concat!(
        include_str!("commands.rs"),
        include_str!("evidence_commands.rs"),
        include_str!("appraisal_commands.rs")
    );
    registered_commands()
        .into_iter()
        .map(|name| {
            let rust = declared_return(src, &name);
            let ts = RETURN_TYPES
                .iter()
                .find(|(r, _)| *r == rust)
                .unwrap_or_else(|| panic!("{name} returns {rust}: add it to RETURN_TYPES"))
                .1;
            (name, json!(ts))
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// A library status with every field set: a folder that cannot be reached,
/// one waiting source with its note, one file left in another folder, and
/// what the launch could not keep (never set beside `unreachable` by the
/// app, #134; set here for its type).
fn library_status() -> LibraryStatus {
    LibraryStatus {
        path: Some("/Volumes/Papers".into()),
        unreachable: Some("the library folder /Volumes/Papers cannot be used".into()),
        counts: PlaceCounts {
            app: 1,
            waiting: 1,
            library: 3,
        },
        waiting: vec![WaitingSource {
            paper_id: "p1".into(),
            title: "Trial 😀".into(),
            kind: SourceKind::Pdf,
            note: Some("the drive is unplugged".into()),
        }],
        elsewhere: vec![WaitingSource {
            paper_id: "p2".into(),
            title: "Cohort".into(),
            kind: SourceKind::Jats,
            note: Some("its file was left in the library folder /Volumes/Old".into()),
        }],
        launch_note: Some(
            "the folder was found at /Volumes/Papers 2, but the app could not save that".into(),
        ),
    }
}

/// `text` as stored, with its own assessment and the full-text check unconfirmed.
fn stored(text: CanonicalText) -> StoredCanonical {
    StoredCanonical {
        id: 1,
        source_id: 1,
        assessment_json: serde_json::to_string(&assess_full_text(&text).unwrap()).unwrap(),
        text,
        confirmed_full_text: false,
        rederive_failure: None,
    }
}

/// The code-point span of `needle` in `text`.
fn span_of(text: &CanonicalText, needle: &str) -> TextSpan {
    let start = text.text[..text.text.find(needle).unwrap()].chars().count();
    TextSpan {
        start,
        end: start + needle.chars().count(),
    }
}

/// A conversation on [`PAPER`] (stored as text 1, the fixture's `paper_view`)
/// with a complete turn citing one located and one unlocated quote (the
/// model's copy in lower case, so the paper's wording shows), a stopped turn
/// cut off inside a quote, and a failed whole-paper turn with its error: every
/// `TurnStatus` and `Mode`, which `api.ts` lists by hand.
fn conversation() -> ConversationView {
    let mut db = open_memory_store().unwrap();
    let p = create_paper(&mut db, &Default::default(), Some("Trial 😀")).unwrap();
    let source = NewSource {
        kind: SourceKind::PlainText,
        origin: "paste".into(),
        file_name: None,
        bytes: PAPER.as_bytes().to_vec(),
        decoded: None,
    };
    let sid = save_source(&mut db, &p, source, SourcePlace::App).unwrap();
    let text = canonical_from_markdown(PAPER);
    let cid = save_canonical(&mut db, &p, sid, &text, &assess_full_text(&text).unwrap()).unwrap();
    let conv = create_conversation(&mut db, &p, cid).unwrap();
    let turn = |answer: &str, status, quotes| NewTurn {
        question: "What?".into(),
        answer: answer.into(),
        status,
        error: None,
        model: "ollama:m".into(),
        mode: Mode::Retrieval,
        passages: vec![span_of(&text, "Body 😀 text.")],
        quotes,
        input_tokens: 1,
        output_tokens: 1,
    };
    let quotes = vec![
        NewQuote {
            number: Some(1),
            quote: "body 😀 text".into(),
            span: Some(span_of(&text, "Body 😀 text.")),
            similarity: 1.0,
            contradiction: None,
        },
        NewQuote {
            number: None,
            quote: "not in it".into(),
            span: None,
            similarity: 0.2,
            contradiction: Some("number".into()),
        },
    ];
    let complete = "It says <q>body 😀 text</q> but not <q>not in it</q>.";
    save_turn(&mut db, conv, &turn(complete, TurnStatus::Complete, quotes)).unwrap();
    let stopped = turn("So far <q>Aft", TurnStatus::Stopped, Vec::new());
    save_turn(&mut db, conv, &stopped).unwrap();
    let failed = NewTurn {
        error: Some("the stream broke off".into()),
        mode: Mode::WholePaper,
        passages: Vec::new(),
        ..turn("Partly", TurnStatus::Failed, Vec::new())
    };
    save_turn(&mut db, conv, &failed).unwrap();
    conversation_view(&mut db, conv).unwrap()
}

/// PICO on [`PAPER`]: first a run its check found unsuitable (read from the
/// whole text, with a note), then a run whose population cites
/// `Body 😀 text.` (the model's copy in lower case), its intervention
/// unverified with a quote not found and one dropped, the rest not reported,
/// condensed with a partial status; the view on it, the unsuitable run's own
/// view, and the view with no run yet. Every `PicoPartState` appears, which
/// `api.ts` lists by hand. Times are pinned, as SQLite stamps the clock's.
fn pico() -> (PicoView, PicoRunView, PicoRunView, PicoView) {
    use bmlib::quality::{PicoCitation, PicoExtraction, PicoPart, PicoPartState, PicoSuitability};
    let mut db = open_memory_store().unwrap();
    let p = create_paper(&mut db, &Default::default(), Some("Trial 😀")).unwrap();
    let source = NewSource {
        kind: SourceKind::PlainText,
        origin: "paste".into(),
        file_name: None,
        bytes: PAPER.as_bytes().to_vec(),
        decoded: None,
    };
    let sid = save_source(&mut db, &p, source, SourcePlace::App).unwrap();
    let text = canonical_from_markdown(PAPER);
    let cid = save_canonical(&mut db, &p, sid, &text, &assess_full_text(&text).unwrap()).unwrap();
    let empty = pico_view(&mut db, &p).unwrap();
    let check = |is_suitable, from_abstract, notes: Vec<String>| PicoCheck {
        suitability: PicoSuitability {
            is_suitable,
            is_intervention_study: Some(is_suitable),
            has_comparison: Some(!is_suitable),
            study_type: Some("narrative review".into()),
            confidence: Some(0.9),
            rationale: Some("Because.".into()),
            condensed_from_chars: None,
            condensation_status: None,
            notes,
        },
        from_abstract,
    };
    let not_suitable = NewPicoRun {
        model: "ollama:m".into(),
        outcome: PicoOutcome::NotSuitable(check(false, false, vec!["n".into()])),
    };
    let unsuitable = save_run(&mut db, &p, cid, &not_suitable).unwrap();
    let span = span_of(&text, "Body 😀 text.");
    let part = |state, citations, not_found: Vec<String>, dropped| PicoPart {
        description: "d".into(),
        state,
        confidence: Some(0.8),
        citations,
        quotes_not_found: not_found,
        quotes_contradicted: vec![],
        quotes_dropped: dropped,
    };
    let cited = vec![PicoCitation {
        start: span.start,
        end: span.end,
        text: "body 😀 text".into(),
        similarity: 1.0,
        exact: false,
    }];
    let extraction = PicoExtraction {
        population: part(PicoPartState::Verified, cited, vec![], 0),
        intervention: part(
            PicoPartState::Unverified,
            vec![],
            vec!["not in it".into()],
            1,
        ),
        comparison: PicoPart {
            quotes_contradicted: vec![bmlib::quality::PicoContradictedQuote {
                quote: "Body 3 text.".into(),
                contradiction: bmlib::citations::PassageContradiction::Negation,
            }],
            ..part(PicoPartState::Contradicted, vec![], vec![], 0)
        },
        outcome: part(PicoPartState::NotReported, vec![], vec![], 0),
        study_type: Some("RCT".into()),
        sample_size: Some("N=105".into()),
        overall_confidence: Some(0.8),
        condensed_from_chars: Some(50000),
        condensation_status: Some("partial".into()),
        notes: vec!["n".into()],
    };
    // Saved before the extracted run, so that one stays the current run.
    let forced = NewPicoRun {
        model: "ollama:m".into(),
        outcome: PicoOutcome::Extracted {
            check: None,
            extraction: extraction.clone(),
        },
    };
    let forced = save_run(&mut db, &p, cid, &forced).unwrap();
    let extracted = NewPicoRun {
        model: "anthropic:claude".into(),
        outcome: PicoOutcome::Extracted {
            check: Some(check(true, true, vec![])),
            extraction,
        },
    };
    save_run(&mut db, &p, cid, &extracted).unwrap();
    bmlib::db::execute(
        &mut db,
        "UPDATE pico_runs SET created_at = '2026-11-03 09:00:00'",
        &[],
    )
    .unwrap();
    (
        pico_view(&mut db, &p).unwrap(),
        pico_run_view(&mut db, unsuitable).unwrap(),
        pico_run_view(&mut db, forced).unwrap(),
        empty,
    )
}

/// PRISMA on [`PAPER`]: first a run its check found not suitable, then a run of
/// three parts: items 1 and 2 done (item 1 cited at `Body 😀 text.` with the
/// model's copy in lower case, item 2 unbacked), items 3 and 4 failed, item 5
/// still pending. The view on it (the first run `earlier`), the incomplete
/// run's own view and the not-suitable run's. Times are pinned.
fn prisma() -> (PrismaView, PrismaRunView, PrismaRunView) {
    use bmlib::quality::prisma2020_models::{Judgement, Suitability};
    use bmlib::quality::{ItemAssessment, PartAssessment, PartPlan, PicoCitation};
    let mut db = open_memory_store().unwrap();
    let p = create_paper(&mut db, &Default::default(), Some("Trial 😀")).unwrap();
    let source = NewSource {
        kind: SourceKind::PlainText,
        origin: "paste".into(),
        file_name: None,
        bytes: PAPER.as_bytes().to_vec(),
        decoded: None,
    };
    let sid = save_source(&mut db, &p, source, SourcePlace::App).unwrap();
    let text = canonical_from_markdown(PAPER);
    let assessment = assess_full_text(&text).unwrap();
    let cid = save_canonical(&mut db, &p, sid, &text, &assessment).unwrap();
    let check = |is_suitable, notes: Vec<String>| PrismaCheck {
        suitability: Suitability {
            is_systematic_review: is_suitable,
            is_meta_analysis: false,
            is_suitable,
            document_type: "systematic review".into(),
            variant: Some("scoping review".into()),
            rationale: "r".into(),
            confidence: 0.5,
            condensed_from_chars: Some(9),
            condensation_status: Some("partial".into()),
            notes,
        },
        from_abstract: !is_suitable,
    };
    let unsuitable =
        save_not_suitable(&mut db, &p, cid, "m", &check(false, vec!["n".into()])).unwrap();
    let plan = |ids: &[&'static str]| PartPlan {
        item_ids: ids.to_vec(),
    };
    let run = create_prisma_run(
        &mut db,
        &p,
        cid,
        "anthropic:claude",
        Some(&check(true, vec!["c".into()])),
        false,
        &[plan(&["1", "2"]), plan(&["3", "4"]), plan(&["5"])],
    )
    .unwrap();
    let parts = pending_parts(&mut db, run).unwrap();
    let [done, failed, _pending] = &parts[..] else {
        panic!("not three parts");
    };
    let start = text.text.find("Body").unwrap();
    let start = text.text[..start].chars().count();
    let cited = vec![PicoCitation {
        start,
        end: start + "Body 😀 text.".chars().count(),
        text: "body 😀 text.".into(),
        similarity: 0.9,
        exact: false,
    }];
    let item = |id, judgement, citations| ItemAssessment {
        id,
        model_judgement: judgement,
        judgement,
        unbacked: false,
        explanation: "e".into(),
        citations,
        quotes_not_found: vec![],
        quotes_contradicted: vec![],
        quotes_dropped: 0,
    };
    let mut unbacked = item("2", Judgement::NotReported, vec![]);
    unbacked.model_judgement = Judgement::Reported;
    unbacked.unbacked = true;
    unbacked.quotes_not_found = vec!["q".into()];
    unbacked.quotes_contradicted = vec![bmlib::quality::PicoContradictedQuote {
        quote: "q2".into(),
        contradiction: bmlib::citations::PassageContradiction::Number,
    }];
    unbacked.quotes_dropped = 1;
    let reply = PartAssessment {
        items: vec![item("1", Judgement::Reported, cited), unbacked],
        missing: vec![],
        unasked: vec!["99".into()],
        condensed_from_chars: Some(9000),
        condensation_status: Some("completed".into()),
        notes: vec!["p".into()],
    };
    save_part(&mut db, done.id, &reply).unwrap();
    fail_part(&mut db, failed.id, "timeout", "too slow").unwrap();
    bmlib::db::execute(
        &mut db,
        "UPDATE prisma_runs SET created_at = '2026-11-03 09:00:00'",
        &[],
    )
    .unwrap();
    (
        prisma_view(&mut db, &p).unwrap(),
        prisma_run_view(&mut db, run).unwrap(),
        prisma_run_view(&mut db, unsuitable).unwrap(),
    )
}

/// Three evidence runs on [`PAPER`]: a trial with *no primary outcome*, one
/// *not covered* (a cohort study), then an assessed trial with two outcomes.
/// Outcome 0 is weighed: every domain answered, its D5 with a claimed fact, a
/// failed and a made second look and an analysis check, its escalation asked.
/// Outcome 1 has no result, a failed D3, its D4 and escalation pending. The
/// view on them (the assessed run current, and the paper's appraisal named),
/// and each run's own view. Then that appraisal through its states, its paired
/// run hidden until the save: *confirming* (the proposals stored), *answering*
/// (D1's 1.1 answered, marked, a source ticked and a note; the machine's D1
/// done and outcome 1's D4 failed) and *saved* (every question answered, D1
/// overridden to high; the machine done but for that D4 and outcome 1's
/// overall, its outcome 0 overall stored as some concerns against its high
/// domains, so a machine departure shows). Times are pinned.
fn evidence() -> (
    (
        EvidenceView,
        EvidenceRunView,
        EvidenceRunView,
        EvidenceRunView,
    ),
    [AppraisalView; 3],
) {
    use bmlib::quality::rob2_agent::d5_assessment;
    use bmlib::quality::rob2_d5_checks::{fact_verdict, D5Fact, D5_FACTS};
    use bmlib::quality::rob2_d5_facts::{AnalysisCheck, D5Facts, FactReport, OtherMeasurement};
    use bmlib::quality::{
        applicable_questions, build_ladder, domain_judgement, Answer, Answers, AssessedOutcome,
        DesignProposal, Domain, DomainAssessment, DomainResult, EffectScale, Escalation, Judgement,
        LadderSettings, Level, MachineAnswer, OutcomeProposal, OutcomeRob2, PicoCitation,
        PicoContradictedQuote, ProposedOutcome, RctLayout, ReportedResult, Rob2Escalation,
        Rob2Step, StudyDesign, DESIGN_PROMPT_VERSION, OUTCOME_PROMPT_VERSION,
    };
    let mut db = open_memory_store().unwrap();
    let p = create_paper(&mut db, &Default::default(), Some("Trial 😀")).unwrap();
    let source = NewSource {
        kind: SourceKind::PlainText,
        origin: "paste".into(),
        file_name: None,
        bytes: PAPER.as_bytes().to_vec(),
        decoded: None,
    };
    let sid = save_source(&mut db, &p, source, SourcePlace::App).unwrap();
    let text = canonical_from_markdown(PAPER);
    let assessment = assess_full_text(&text).unwrap();
    let cid = save_canonical(&mut db, &p, sid, &text, &assessment).unwrap();
    let start = text.text.find("Body").unwrap();
    let start = text.text[..start].chars().count();
    let cited = PicoCitation {
        start,
        end: start + "Body 😀 text.".chars().count(),
        text: "body 😀 text.".into(),
        similarity: 0.9,
        exact: false,
    };
    let contradicted = vec![PicoContradictedQuote {
        quote: "q2".into(),
        contradiction: bmlib::citations::PassageContradiction::Number,
    }];
    let design = |design, rct_layout| DesignProposal {
        design,
        rct_layout,
        citations: vec![cited.clone()],
        quotes_not_found: vec!["q".into()],
        contradicted: contradicted.clone(),
        quotes_dropped: 1,
        reasoning: "r".into(),
        condensed: false,
        prompt_version: DESIGN_PROMPT_VERSION,
    };
    let no_outcome = create_evidence_run(&mut db, &p, cid, "m", None).unwrap();
    let step = pending_steps(&mut db, no_outcome).unwrap()[0].id;
    let trial = design(StudyDesign::Rct, Some(RctLayout::Parallel));
    save_design(&mut db, step, &trial).unwrap();
    let step = pending_steps(&mut db, no_outcome).unwrap()[0].id;
    let none = OutcomeProposal {
        outcomes: vec![],
        condensed: false,
        prompt_version: OUTCOME_PROMPT_VERSION,
    };
    save_outcomes(&mut db, step, &none).unwrap();
    let not_covered = create_evidence_run(&mut db, &p, cid, "m", None).unwrap();
    let step = pending_steps(&mut db, not_covered).unwrap()[0].id;
    save_design(&mut db, step, &design(StudyDesign::CohortProspective, None)).unwrap();
    let run = create_evidence_run(&mut db, &p, cid, "anthropic:claude", None).unwrap();
    let weighed = ProposedOutcome {
        name: "pain".into(),
        timepoint: Some("12 weeks".into()),
        result: Some(ReportedResult {
            measure: "risk ratio".into(),
            scale: EffectScale::Ratio,
            estimate: 0.7,
            ci_low: Some(0.55),
            ci_high: Some(0.89),
            ci_level: Some(0.95),
            n_analysed: Some(1200),
        }),
        result_error: None,
        citations: vec![cited.clone()],
        quotes_not_found: vec![],
        contradicted: vec![],
        quotes_dropped: 0,
    };
    let unweighed = ProposedOutcome {
        name: "function".into(),
        timepoint: None,
        result: None,
        result_error: Some("no estimate".into()),
        ..weighed.clone()
    };
    let assessed = |domain| {
        let mut given = Answers::new();
        loop {
            let next: Vec<&str> = applicable_questions(domain, &given)
                .into_iter()
                .filter(|q| !given.contains_key(*q))
                .collect();
            if next.is_empty() {
                break;
            }
            for q in next {
                given.insert(q.to_string(), Answer::Y);
            }
        }
        let judgement = domain_judgement(domain, &given).unwrap();
        let answers = given
            .iter()
            .map(|(q, a)| {
                let machine = MachineAnswer {
                    answer: *a,
                    citations: vec![cited.clone()],
                    quotes_not_found: vec![],
                    contradicted: vec![],
                    quotes_dropped: 0,
                    reasoning: "a".into(),
                };
                (q.clone(), machine)
            })
            .collect();
        DomainResult::Assessed(DomainAssessment {
            domain,
            answers,
            judgement,
            condensed: false,
            ignored: vec![],
            facts: None,
        })
    };
    let d5 = || {
        let facts = D5_FACTS
            .iter()
            .map(|&fact| {
                let claimed = fact == D5Fact::MeasureFixed;
                let citations: Vec<PicoCitation> =
                    [cited.clone()].into_iter().filter(|_| claimed).collect();
                let found: Vec<&str> = citations.iter().map(|c| c.text.as_str()).collect();
                let report = FactReport {
                    claimed,
                    verdict: fact_verdict(fact, claimed, &found, &weighed.name),
                    citations,
                    quotes_not_found: vec![],
                    contradicted: vec![],
                    quotes_dropped: 0,
                };
                (fact, report)
            })
            .collect();
        let failed = OtherMeasurement {
            label: "at 6 weeks".into(),
            named: vec![cited.clone()],
            result: vec![cited.clone()],
            second_look: None,
            second_look_error: Some("busy".into()),
            second_look_retryable: false,
            second_look_status: None,
            quotes_not_found: vec!["q".into()],
            contradicted: contradicted.clone(),
            quotes_dropped: 1,
        };
        let looked = OtherMeasurement {
            label: "at 3 weeks".into(),
            second_look: Some(vec![cited.clone()]),
            second_look_error: None,
            ..failed.clone()
        };
        DomainResult::Assessed(d5_assessment(D5Facts {
            outcome: weighed.name.clone(),
            facts,
            measurements: vec![failed, looked],
            analysis: AnalysisCheck {
                differs: true,
                planned: vec![cited.clone()],
                reported: vec![cited.clone()],
                quotes_not_found: vec![],
                contradicted: vec![],
                quotes_dropped: 0,
            },
        }))
    };
    let asked = Rob2Escalation::Asked(Escalation {
        escalate: false,
        reasoning: "e".into(),
    });
    let d3_of_1 = EvidenceStep::Rob2(Rob2Step::Domain {
        domain: Domain::D3,
        outcome: Some(1),
    });
    let d4_of_1 = EvidenceStep::Rob2(Rob2Step::Domain {
        domain: Domain::D4,
        outcome: Some(1),
    });
    let escalation_of_1 = EvidenceStep::Rob2(Rob2Step::Escalation { outcome: 1 });
    while let Some(step) = pending_steps(&mut db, run)
        .unwrap()
        .into_iter()
        .find(|s| s.step != escalation_of_1 && s.step != d4_of_1)
    {
        match step.step {
            EvidenceStep::Design => {
                let d = design(StudyDesign::Rct, Some(RctLayout::Parallel));
                save_design(&mut db, step.id, &d).unwrap();
            }
            EvidenceStep::Outcomes => {
                let o = OutcomeProposal {
                    outcomes: vec![weighed.clone(), unweighed.clone()],
                    condensed: true,
                    prompt_version: OUTCOME_PROMPT_VERSION,
                };
                save_outcomes(&mut db, step.id, &o).unwrap();
            }
            s if s == d3_of_1 => {
                let failed = DomainResult::Failed {
                    domain: Domain::D3,
                    reason: "unreadable".into(),
                    retryable: false,
                    status: None,
                };
                save_domain(&mut db, step.id, &failed).unwrap();
            }
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain: Domain::D5, ..
            }) => save_domain(&mut db, step.id, &d5()).unwrap(),
            EvidenceStep::Rob2(Rob2Step::Domain { domain, .. }) => {
                save_domain(&mut db, step.id, &assessed(domain)).unwrap();
            }
            EvidenceStep::Rob2(Rob2Step::Escalation { .. }) => {
                let overall = Judgement::SomeConcerns;
                let ladder = build_ladder(
                    Level::High,
                    overall,
                    weighed.result.as_ref(),
                    &LadderSettings::default(),
                )
                .unwrap();
                let outcome = AssessedOutcome {
                    outcome: weighed.clone(),
                    rob2: OutcomeRob2 {
                        outcome: weighed.name.clone(),
                        domains: vec![],
                        escalation: Some(Escalation {
                            escalate: false,
                            reasoning: "e".into(),
                        }),
                        escalation_error: None,
                        escalation_retryable: false,
                        escalation_status: None,
                        overall: Some(overall),
                    },
                    ladder: Some(ladder),
                    ladder_error: None,
                };
                save_escalation(&mut db, step.id, &asked, &outcome).unwrap();
            }
        }
    }
    bmlib::db::execute(
        &mut db,
        "UPDATE evidence_runs SET created_at = '2026-11-25 09:00:00'",
        &[],
    )
    .unwrap();
    bmlib::db::execute(
        &mut db,
        "UPDATE evidence_runs SET first_shown_at = '2026-11-25 09:05:00' WHERE id = ?",
        &[bmlib::db::Value::from(run)],
    )
    .unwrap();
    // The reader's appraisal, not blind (a run was shown), so the tab's view
    // names it; its paired run is hidden until the save, so the views of the
    // runs above are unchanged.
    let aid = start_appraisal(&mut db, &p).unwrap();
    let views = (
        evidence_view(&mut db, &p).unwrap(),
        evidence_run_view(&mut db, run).unwrap(),
        evidence_run_view(&mut db, not_covered).unwrap(),
        evidence_run_view(&mut db, no_outcome).unwrap(),
    );
    let paired = create_evidence_run(&mut db, &p, cid, "anthropic:claude", Some(aid)).unwrap();
    // Every time is pinned before a view is taken.
    let pin = |db: &mut dyn bmlib::db::Db| {
        for sql in [
            "UPDATE evidence_runs SET created_at = '2026-11-25 09:00:00'",
            "UPDATE evidence_appraisals SET started_at = '2026-11-26 10:00:00',
               confirmed_at = CASE WHEN confirmed_at IS NULL THEN NULL ELSE '2026-11-26 10:05:00' END,
               revealed_at = CASE WHEN revealed_at IS NULL THEN NULL ELSE '2026-11-26 10:30:00' END,
               saved_at = CASE WHEN saved_at IS NULL THEN NULL ELSE '2026-11-26 10:30:00' END",
            "UPDATE evidence_appraisal_versions SET created_at = '2026-11-26 10:00:00',
               frozen_at = CASE WHEN frozen_at IS NULL THEN NULL ELSE '2026-11-26 10:30:00' END",
        ] {
            bmlib::db::execute(db, sql, &[]).unwrap();
        }
    };
    let viewed = |db: &mut rusqlite::Connection| {
        pin(db);
        appraisal_view(db, aid, None, false).unwrap()
    };
    while let Some(step) = pending_steps(&mut db, paired).unwrap().into_iter().next() {
        match step.step {
            EvidenceStep::Design => {
                let d = design(StudyDesign::Rct, Some(RctLayout::Parallel));
                save_design(&mut db, step.id, &d).unwrap();
            }
            _ => {
                let o = OutcomeProposal {
                    outcomes: vec![weighed.clone(), unweighed.clone()],
                    condensed: false,
                    prompt_version: OUTCOME_PROMPT_VERSION,
                };
                save_outcomes(&mut db, step.id, &o).unwrap();
            }
        }
    }
    let confirming = viewed(&mut db);
    let confirmed = |i: usize, o: &ProposedOutcome| ConfirmedOutcomeInput {
        from_proposed: Some(i),
        name: o.name.clone(),
        timepoint: o.timepoint.clone(),
        result: o.result.clone(),
    };
    let input = ConfirmInput {
        design: "rct".into(),
        rct_layout: Some("parallel".into()),
        outcomes: vec![confirmed(0, &weighed), confirmed(1, &unweighed)],
    };
    confirm(&mut db, aid, &input).unwrap();
    // The machine's pending steps saved (the first only, with `only_d1`), but
    // outcome 1's overall, which waits for its failed D4.
    let machine = |db: &mut rusqlite::Connection, only_d1: bool| {
        while let Some(step) = pending_steps(db, paired)
            .unwrap()
            .into_iter()
            .find(|s| s.step != escalation_of_1)
        {
            match step.step {
                EvidenceStep::Rob2(Rob2Step::Domain {
                    domain: Domain::D5, ..
                }) => save_domain(db, step.id, &d5()).unwrap(),
                EvidenceStep::Rob2(Rob2Step::Domain { domain, .. }) => {
                    save_domain(db, step.id, &assessed(domain)).unwrap();
                }
                _ => {
                    let ladder = build_ladder(
                        Level::High,
                        Judgement::SomeConcerns,
                        weighed.result.as_ref(),
                        &LadderSettings::default(),
                    )
                    .unwrap();
                    let outcome = AssessedOutcome {
                        outcome: weighed.clone(),
                        rob2: OutcomeRob2 {
                            outcome: weighed.name.clone(),
                            domains: vec![],
                            escalation: Some(Escalation {
                                escalate: false,
                                reasoning: "e".into(),
                            }),
                            escalation_error: None,
                            escalation_retryable: false,
                            escalation_status: None,
                            overall: Some(Judgement::SomeConcerns),
                        },
                        ladder: Some(ladder),
                        ladder_error: None,
                    };
                    save_escalation(db, step.id, &asked, &outcome).unwrap();
                }
            }
            if only_d1 {
                break;
            }
        }
    };
    machine(&mut db, true);
    let d4 = pending_steps(&mut db, paired)
        .unwrap()
        .into_iter()
        .find(|s| s.step == d4_of_1)
        .unwrap();
    let failed = StepFailure {
        message: PAIRED_STEP_FAILED.into(),
        retryable: true,
        status: Some(503),
    };
    fail_step(&mut db, d4.id, &failed).unwrap();
    // The reader: 1.1 answered, its sentence marked and the protocol used,
    // and a note on D1.
    let d1 = |q: &str, a| AppraisalEdit::Answer {
        domain: Domain::D1,
        outcome: None,
        question_id: q.into(),
        answer: Some(a),
    };
    for e in [
        d1("1.1", Answer::Y),
        AppraisalEdit::Mark {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            span: Some(TextSpan {
                start: cited.start,
                end: cited.end,
            }),
        },
        AppraisalEdit::Sources {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            protocol: true,
            registry: false,
            supplement: false,
        },
        AppraisalEdit::Note {
            domain: Domain::D1,
            outcome: None,
            note: Some("Blocks of four.".into()),
        },
    ] {
        edit(&mut db, aid, &e).unwrap();
    }
    let answering = viewed(&mut db);
    // The machine goes on; the reader answers every question low, overrides
    // D1 to high (a departure), and saves.
    machine(&mut db, false);
    let mut edits = vec![d1("1.2", Answer::Y), d1("1.3", Answer::N)];
    for o in 0..2 {
        for (domain, q, a) in [
            (Domain::D2, "2.1", Answer::N),
            (Domain::D2, "2.2", Answer::N),
            (Domain::D2, "2.6", Answer::Y),
            (Domain::D3, "3.1", Answer::Y),
            (Domain::D4, "4.1", Answer::N),
            (Domain::D4, "4.2", Answer::N),
            (Domain::D4, "4.3", Answer::N),
            (Domain::D5, "5.1", Answer::Y),
            (Domain::D5, "5.2", Answer::N),
            (Domain::D5, "5.3", Answer::N),
        ] {
            edits.push(AppraisalEdit::Answer {
                domain,
                outcome: Some(o),
                question_id: q.into(),
                answer: Some(a),
            });
        }
    }
    edits.push(AppraisalEdit::Override {
        domain: Domain::D1,
        outcome: None,
        judgement: Some(Judgement::High),
    });
    for e in edits {
        edit(&mut db, aid, &e).unwrap();
    }
    save(&mut db, aid).unwrap();
    let saved = viewed(&mut db);
    (views, [confirming, answering, saved])
}

/// Builds every value the fixture pins, plus `span_texts`: the title's,
/// then each heading's and block's text in document order, cut by code point
/// in Rust, for the frontend to compare its UTF-16 slices with.
fn contract() -> Value {
    let canonical = canonical_from_markdown(PAPER);
    let decoded = Decoded {
        encoding: "windows-1252".into(),
        guessed: true,
    };
    let view = paper_view(
        "p1",
        "Trial 😀",
        &stored(canonical.clone()),
        SourceKind::PlainText,
        Some(&decoded),
    )
    .unwrap();
    let bare = paper_view(
        "p2",
        "",
        &stored(canonical_from_markdown(BARE_PAPER)),
        SourceKind::Pdf,
        None,
    )
    .unwrap();
    let spans = canonical
        .title_span
        .into_iter()
        .chain(canonical.sections.iter().flat_map(|s| {
            s.heading_span
                .into_iter()
                .chain(s.blocks.iter().map(|b| b.span))
        }));
    let span_texts: Vec<&str> = spans.map(|s| canonical.span_text(s).unwrap()).collect();
    let summary = PaperSummary {
        id: "p1".into(),
        title: "Trial 😀".into(),
        identifiers: Identifiers {
            pmid: Some("123".into()),
            doi: Some("10.1/x".into()),
            pmc_id: None,
        },
        opened_at: "2026-10-31 09:00:00".into(),
    };
    let errors: Vec<CommandError> = EVERY_KIND
        .iter()
        .map(|&kind| CommandError {
            kind,
            message: "m".into(),
        })
        .collect();
    let conversation = conversation();
    let (pico_view, pico_run_not_suitable, pico_run_forced, pico_view_empty) = pico();
    let (prisma_view, prisma_run_incomplete, prisma_run_not_suitable) = prisma();
    let (
        (
            evidence_view,
            evidence_run_assessed,
            evidence_run_not_covered,
            evidence_run_no_primary_outcome,
        ),
        [appraisal_confirming, appraisal_answering, appraisal_saved],
    ) = evidence();
    let interrogator = InterrogatorView {
        canonical_id: 2,
        current: None,
        earlier: vec![ConversationSummary {
            id: conversation.id,
            canonical_id: conversation.canonical_id,
            turns: conversation.turns.len(),
            created_at: "2026-11-02 09:00:00".into(),
        }],
    };
    let settings = LlmSettings {
        model: Some("anthropic:claude".into()),
        context_tokens: Some(8192),
        // One provider per key source (keychain, environment, none), so the
        // frontend's list of sources is checked against every value.
        providers: vec![
            ProviderInfo {
                name: "anthropic".into(),
                default_model: "claude".into(),
                needs_key: true,
                has_key: true,
                key_source: Some(KeySource::Keychain),
                key_env: Some("ANTHROPIC_API_KEY".into()),
            },
            ProviderInfo {
                name: "openai".into(),
                default_model: "gpt".into(),
                needs_key: true,
                has_key: true,
                key_source: Some(KeySource::Env),
                key_env: Some("OPENAI_API_KEY".into()),
            },
            ProviderInfo {
                name: "ollama".into(),
                default_model: "llama".into(),
                needs_key: false,
                has_key: false,
                key_source: None,
                key_env: None,
            },
        ],
        keychain_error: Some("k".into()),
    };
    json!({
        "turn_view": conversation.turns[0],
        "turn_view_stopped": conversation.turns[1],
        "turn_view_failed": conversation.turns[2],
        "conversation_view": conversation,
        "interrogator_view": interrogator,
        "pico_view": pico_view,
        "pico_run_not_suitable": pico_run_not_suitable,
        "pico_run_forced": pico_run_forced,
        "pico_view_empty": pico_view_empty,
        "prisma_view": prisma_view,
        "prisma_run_incomplete": prisma_run_incomplete,
        "prisma_run_not_suitable": prisma_run_not_suitable,
        "evidence_view": evidence_view,
        "evidence_run_assessed": evidence_run_assessed,
        "evidence_run_not_covered": evidence_run_not_covered,
        "evidence_run_no_primary_outcome": evidence_run_no_primary_outcome,
        "appraisal_confirming": appraisal_confirming,
        "appraisal_answering": appraisal_answering,
        "appraisal_saved": appraisal_saved,
        "damaged_appraisal": DamagedAppraisal {
            id: 3,
            error: "appraisal 3: \"renamed_design\" is not a design".into(),
        },
        "llm_settings": settings,
        "embedder_status": EmbedderStatus {
            downloaded: false,
            size_bytes: EMBEDDING_MODEL.size,
        },
        "granted_file": GrantedFile { id: 1, name: "paper.pdf".into() },
        "files_dropped_event": FILES_DROPPED_EVENT,
        "handoff_event": HANDOFF_EVENT,
        "library_settled_event": LIBRARY_SETTLED_EVENT,
        "library_settled": LibrarySettled { moved: 2, left: 1, error: None },
        "library_status": library_status(),
        "library_status_empty": LibraryStatus {
            path: None,
            unreachable: None,
            counts: PlaceCounts::default(),
            waiting: vec![],
            elsewhere: vec![],
            launch_note: None,
        },
        "library_listing": Listing {
            papers: vec![ListedPaper {
                folder: "2024 Trial - Tablets (PMID 7)".into(),
                title: Some("Trial 😀".into()),
                files: vec![ListedFile {
                    name: "pdf-3f9a1c2e7b40.pdf".into(),
                    size: Some(1234),
                    used_by: vec!["MedicalFactChecker on a Mac".into()],
                    used_here: true,
                }],
                used_by: vec!["MedicalFactChecker on a Mac".into()],
                used_here: true,
                users_unknown: true,
            }],
            problems: vec!["UnrecordedFile { folder: \"x\", name: \"y\" }".into()],
        },
        "library_listing_empty": Listing {
            papers: vec![ListedPaper {
                folder: "Untitled (PMID 8)".into(),
                title: None,
                files: vec![ListedFile {
                    name: "text-000000000000.txt".into(),
                    size: None,
                    used_by: vec![],
                    used_here: false,
                }],
                used_by: vec![],
                used_here: false,
                users_unknown: false,
            }],
            problems: vec![],
        },
        "changed": Changed {
            copied: 1,
            moved: 2,
            failed: vec![PaperProblem {
                paper_id: "p1".into(),
                title: "Trial 😀".into(),
                why: "the file is gone".into(),
            }],
        },
        "handoff_summary": HandoffSummary {
            id: 1,
            received_at: "2026-11-04 09:00:00".into(),
            sender: Some("MedicalFactChecker".into()),
            title: Some("Trial 😀".into()),
            identifiers: Identifiers {
                pmid: Some("123".into()),
                doi: None,
                pmc_id: Some("PMC9".into()),
            },
            has_full_text: true,
            refusal: None,
        },
        "handoff_summary_refused": HandoffSummary {
            id: 2,
            received_at: "2026-11-04 09:01:00".into(),
            sender: None,
            title: None,
            identifiers: Identifiers::default(),
            has_full_text: false,
            refusal: Some("This is not a StudyAnalyzer hand-off file.".into()),
        },
        "download_progress": DownloadProgress { done: 1, total: 2 },
        "answer_indexing": AnswerEvent::Indexing { done: 1, total: 2 },
        "answer_delta": AnswerEvent::Delta { text: "x".into() },
        "answer_checking_model": AnswerEvent::CheckingModel,
        "answer_waiting": AnswerEvent::Waiting,
        "pico_condensing": PicoEvent::Condensing { batch: 1, batches: 2, level: 0 },
        "pico_condensed": PicoEvent::Condensed,
        "pico_other": PicoEvent::Other { stage: "s".into(), message: "m".into() },
        "prisma_part": PrismaEvent::Part { index: 1, of: 3, label: "Methods 5-13c".into() },
        "prisma_condensing": PrismaEvent::Condensing { batch: 1, batches: 2, level: 0 },
        "prisma_condensed": PrismaEvent::Condensed,
        "prisma_rechecking": PrismaEvent::Rechecking { item: "7".into() },
        "prisma_other": PrismaEvent::Other { stage: "s".into(), message: "m".into() },
        "evidence_step": EvidenceEvent::Step { label: "Study design".into() },
        "evidence_condensing": EvidenceEvent::Condensing { batch: 1, batches: 2, level: 0 },
        "evidence_condensed": EvidenceEvent::Condensed,
        "evidence_other": EvidenceEvent::Other { stage: "s".into(), message: "m".into() },
        "appraisal_event_progress": AppraisalEvent::Progress { done: 6, total: 11 },
        "open_ready": OpenOutcome::Ready {
            paper_id: "p1".into(),
            notices: vec!["n".into()],
        },
        "open_needs_full_text": OpenOutcome::NeedsFullText {
            paper_id: "p1".into(),
            reason: "r".into(),
            unreachable: true,
        },
        "paper_summary": summary,
        "paper_view": view,
        "paper_view_bare": bare,
        "span_texts": span_texts,
        "command_errors": errors,
        "commands": commands(),
        "returns": returns(),
    })
}

/// The values serialise exactly as the committed fixture. After a deliberate
/// change, run with `UPDATE_CONTRACT` set (to any value) to rewrite it; the
/// test then fails once so the rewrite is never a silent pass. Rerun without
/// it, then make `pnpm run check:web` and `pnpm run test:web` pass.
#[test]
fn json_matches_the_fixture() {
    EVERY_KIND.iter().for_each(|&k| every_kind_is_listed(k));
    every_block_kind_is_mirrored(BlockKind::Paragraph);
    let actual = contract();
    if std::env::var_os("UPDATE_CONTRACT").is_some() {
        let pretty = serde_json::to_string_pretty(&actual).unwrap();
        std::fs::write(FIXTURE, pretty + "\n").unwrap();
        panic!("rewrote {FIXTURE}: review its diff, then rerun without UPDATE_CONTRACT");
    }
    let committed: Value =
        serde_json::from_str(&std::fs::read_to_string(FIXTURE).unwrap()).unwrap();
    assert_eq!(
        actual, committed,
        "the frontend's JSON changed: rerun with UPDATE_CONTRACT=1, then update src/lib/api.ts"
    );
}

/// The app's own origin as Tauri sets it per platform. A URL Tauri does not
/// treat as local is a remote origin, which the ACL refuses for app commands.
const APP_ORIGIN: &str = if cfg!(any(windows, target_os = "android")) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

/// Invokes `cmd` with `args` through the mock runtime, which has no
/// `AppState`, so a command that decodes its arguments fails with a
/// `CommandError` (an object) and touches nothing; one that cannot decode
/// them, or that is not registered, fails with Tauri's message (a string).
fn invoke(cmd: &str, args: Value) -> Result<Value, Value> {
    let app = crate::handler_app();
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    tauri::test::get_ipc_response(
        &webview,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: APP_ORIGIN.parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize().unwrap())
}

/// A value of the JSON type the fixture names for argument `key` of `cmd`.
/// `"number"` is always `1`, so a Rust integer narrowed to another that still
/// holds 1 is not caught; an `"object"` is one the argument's Rust type reads.
fn sample(cmd: &str, key: &str, kind: &str) -> Value {
    match kind {
        "object" => match (cmd, key) {
            ("confirm_appraisal", "input") => {
                json!({"design": "rct", "rct_layout": "parallel", "outcomes": []})
            }
            ("edit_appraisal", "edit") => {
                json!({"kind": "note", "domain": "D1", "outcome": null, "note": "n"})
            }
            other => panic!("no object sample for {other:?}"),
        },
        "string" => json!("x"),
        "number" => json!(1),
        "boolean" => json!(true),
        // A channel id as `@tauri-apps/api`'s `Channel` serialises one
        // (Tauri's `IPC_PAYLOAD_PREFIX`, which it does not export).
        "channel" => json!("__CHANNEL__:1"),
        other => panic!("no sample for JSON type {other}"),
    }
}

/// A value that is not of the JSON type the fixture names.
fn wrong_sample(kind: &str) -> Value {
    match kind {
        "string" | "channel" | "object" => json!(1),
        "number" | "boolean" => json!("x"),
        other => panic!("no sample for JSON type {other}"),
    }
}

/// The arguments `api.ts` sends for `cmd`, built from `keys` (key → JSON type).
fn arguments(cmd: &str, keys: &Value) -> serde_json::Map<String, Value> {
    keys.as_object()
        .unwrap()
        .iter()
        .map(|(k, kind)| (k.clone(), sample(cmd, k, kind.as_str().unwrap())))
        .collect()
}

/// The arguments Rust takes as an `Option` (command, key): `api.ts` sends a
/// number or null, and Tauri reads a missing key as `None` too.
const OPTIONAL_ARGS: [(&str, &str); 3] = [
    ("run_prisma", "continueRun"),
    ("run_evidence", "continueRun"),
    ("appraisal", "version"),
];

/// Each command, given the keys and types `api.ts` sends, gets past argument
/// decoding, and each key is one Rust requires with that type: dropping it,
/// or sending another JSON type, is refused. So renaming, removing or
/// retyping a Rust parameter, or making it optional, fails here; a key in
/// [`OPTIONAL_ARGS`] must take null and no value instead.
#[test]
fn every_command_decodes_its_arguments() {
    for (cmd, keys) in commands().as_object().unwrap() {
        let args = arguments(cmd, keys);
        if let Err(e) = invoke(cmd, Value::Object(args.clone())) {
            assert!(e.get("kind").is_some(), "{cmd} refused its arguments: {e}");
        }
        for (key, kind) in keys.as_object().unwrap() {
            let mut missing = args.clone();
            missing.remove(key);
            if OPTIONAL_ARGS.contains(&(cmd.as_str(), key.as_str())) {
                // An `Option` takes a missing key or null as `None`; `api.ts` sends null.
                let mut null = args.clone();
                null.insert(key.clone(), Value::Null);
                for args in [missing, null] {
                    if let Err(e) = invoke(cmd, Value::Object(args)) {
                        assert!(e.get("kind").is_some(), "{cmd} refused no {key}: {e}");
                    }
                }
            } else {
                let e = invoke(cmd, Value::Object(missing)).unwrap_err();
                assert!(e.is_string(), "{cmd} ran without {key}: {e}");
            }
            let mut wrong = args.clone();
            wrong.insert(key.clone(), wrong_sample(kind.as_str().unwrap()));
            let e = invoke(cmd, Value::Object(wrong)).unwrap_err();
            assert!(
                e.is_string(),
                "{cmd} took a {key} that is not a {kind}: {e}"
            );
        }
    }
}

/// The checks above can fail: a missing key or an unknown command is Tauri's
/// string, not a `CommandError`.
#[test]
fn a_missing_key_or_command_is_tauris_error() {
    assert!(invoke("open_file", json!({})).unwrap_err().is_string());
    assert!(invoke("no_such_command", json!({}))
        .unwrap_err()
        .is_string());
}

/// The command names `handler()` in `lib.rs` registers, read from its source.
fn registered_commands() -> Vec<String> {
    let src = include_str!("lib.rs");
    let start = src.find("generate_handler![").unwrap() + "generate_handler![".len();
    let end = start + src[start..].find(']').unwrap();
    src[start..end]
        .split(',')
        .map(|c| c.trim().rsplit("::").next().unwrap_or_default().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// The return types are read from the signatures as rustfmt writes them, and
/// a handler that is missing or returns another error type is refused.
#[test]
fn declared_returns_are_read_from_signatures() {
    let src = "pub async fn a<R: Runtime>(\n    app: AppHandle<R>,\n) -> Result<Vec<PaperSummary>, CommandError> {\n\
               pub fn b<R: Runtime>(app: AppHandle<R>) {\n\
               pub fn c<R: Runtime>(app: AppHandle<R>) -> Result<(u8, u8), CommandError> {";
    assert_eq!(declared_return(src, "a"), "Vec<PaperSummary>");
    assert_eq!(declared_return(src, "b"), "()");
    assert_eq!(declared_return(src, "c"), "(u8, u8)");
    for bad in ["d", "e"] {
        let src = "pub fn e<R: Runtime>() -> Result<(), String> {";
        assert!(std::panic::catch_unwind(|| declared_return(src, bad)).is_err());
    }
}

/// [`commands`] lists every command `handler()` registers, so a new one
/// cannot skip the decoding check above.
#[test]
fn every_registered_command_is_listed() {
    let mut registered = registered_commands();
    registered.sort();
    let listed: Vec<String> = commands().as_object().unwrap().keys().cloned().collect();
    assert_eq!(registered, listed, "add the command to commands() here");
}
