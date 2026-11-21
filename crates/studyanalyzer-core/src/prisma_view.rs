//! The PRISMA tab's views: stored runs as the serialisable shapes the
//! frontend shows, spans in UTF-16 units. The PICO tab's counterparts live in
//! [`crate::view`]; this is their sibling, in its own file for size.
//!
//! Every run view holds all 42 checklist items in [`CHECKLIST`] order. An
//! item the run has no shown row for (pending, failed, or a run that stopped
//! at its check) has `judgement: None`, which the tab shows as *Not assessed*.

use std::collections::{BTreeMap, HashMap, HashSet};

use bmlib::db::Db;
use bmlib::fulltext::canonical::TextSpan;
use bmlib::quality::prisma2020_checklist::{item as checklist_item, CHECKLIST, KEY_ITEMS};
use bmlib::quality::prisma2020_models::Judgement;
use bmlib::quality::PartPlan;
use serde::Serialize;

use crate::prisma_store::{
    latest_run, run as stored_run, runs, PrismaCheck, PrismaRunSummary, StoredPrismaItem,
    PART_FAILED, RUN_COMPLETE,
};
use crate::store::{latest_canonical_id, StoreError};
use crate::view::{span_to_utf16, utf16_offsets, ContradictedQuoteView, Utf16Span};

/// The count key for items with no shown judgement.
pub const NOT_ASSESSED: &str = "not_assessed";

/// Every judgement string, in the order the checklist's legend lists them.
/// Each section's `counts` has exactly these keys plus [`NOT_ASSESSED`],
/// zeros included, so the frontend never meets a missing key.
const JUDGEMENTS: [Judgement; 5] = [
    Judgement::Reported,
    Judgement::PartlyReported,
    Judgement::NotReported,
    Judgement::ReportedElsewhere,
    Judgement::NotApplicable,
];

/// A quote of an item that was located in the paper.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaCitationView {
    /// Where the quote sits in the paper's text (UTF-16 units).
    pub span: Utf16Span,
    /// The paper's wording at `span`, never the model's copy.
    pub text: String,
    /// `find_passage`'s similarity, 0-1.
    pub similarity: f64,
    /// Whether the match was exact after normalisation.
    pub exact: bool,
}

/// One checklist item as the tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaItemView {
    /// The checklist id, for example `"13d"`.
    pub id: String,
    /// The item's section as its wire string (`title`, `abstract`,
    /// `introduction`, `methods`, `results`, `discussion` or
    /// `other_information`); the frontend maps it to a heading.
    pub section: String,
    /// The item's short name, from bmlib's checklist.
    pub name: String,
    /// Whether the item is one of the ten key items.
    pub key: bool,
    /// The judgement shown, or `None` for *Not assessed*.
    pub judgement: Option<String>,
    /// What the model said, set whenever `judgement` is; it differs from
    /// `judgement` only for an `unbacked` claim. `None` with it.
    pub model_judgement: Option<String>,
    /// True when the model's claim was shown as `not_reported` for want of
    /// a located quote.
    pub unbacked: bool,
    /// Whether the paper was condensed for this item's answer.
    pub condensed: bool,
    /// Whether a key-item re-check replaced the first answer.
    pub rechecked: bool,
    /// Whether the latest re-check's paper was condensed for it; `false`
    /// when there was none. A re-check on Continue is sized from the parts
    /// left, so it can get the whole paper.
    pub recheck_condensed: bool,
    /// The judgement before the re-check, when there was one.
    pub first_judgement: Option<String>,
    /// The model's explanation.
    pub explanation: Option<String>,
    /// The paper's spans backing the item.
    pub citations: Vec<PrismaCitationView>,
    /// The model's quotes that were not found in the paper, verbatim.
    pub quotes_not_found: Vec<String>,
    /// The model's quotes the paper contradicts, with why.
    pub quotes_contradicted: Vec<ContradictedQuoteView>,
    /// How many of the model's quotes were not usable text at all.
    pub quotes_dropped: usize,
}

/// One section's tally of its items.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaSectionView {
    /// The section's wire string, as in [`PrismaItemView::section`].
    pub section: String,
    /// Items per shown judgement string, plus [`NOT_ASSESSED`]; zeros
    /// included.
    pub counts: BTreeMap<String, usize>,
}

/// How many of the ten key items are gaps.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyGapsView {
    /// The key item ids, in `KEY_ITEMS` order.
    pub items: Vec<String>,
    /// Key items shown `not_reported` (unbacked included) or
    /// `partly_reported`.
    pub gaps: usize,
    /// Key items shown `reported_elsewhere`.
    pub elsewhere: usize,
    /// The number of key items.
    pub total: usize,
    /// Whether the run is complete; a count over an unfinished run is partial.
    pub complete: bool,
}

/// A part that failed, for the tab to name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FailedPartView {
    /// The part's items, as `"Methods 5-13c"`.
    pub label: String,
    /// The error's kind.
    pub error_kind: Option<String>,
    /// The error's message.
    pub error_message: Option<String>,
}

/// A suitability check as the tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaSuitabilityView {
    /// Whether the paper is a systematic review.
    pub is_systematic_review: bool,
    /// Whether the paper is a meta-analysis.
    pub is_meta_analysis: bool,
    /// Whether it is either, so the checklist applies.
    pub is_suitable: bool,
    /// The document type as the model named it.
    pub document_type: String,
    /// The review variant as the model named it.
    pub variant: Option<String>,
    /// The model's explanation.
    pub rationale: String,
    /// The model's confidence, 0-1.
    pub confidence: f64,
    /// The text's length in characters when it was condensed first.
    pub condensed_from_chars: Option<usize>,
    /// How the condensation ended, when it did not complete cleanly.
    pub condensation_status: Option<String>,
}

/// A stored PRISMA run as the tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaRunView {
    /// The run's id.
    pub id: i64,
    /// The paper text its spans point into.
    pub canonical_id: i64,
    /// The `provider:model` that ran it.
    pub model: String,
    /// When it was stored (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// True when the user forced the run past the check.
    pub forced: bool,
    /// `incomplete` or `complete`.
    pub status: String,
    /// The check, or `None` for a forced run with none.
    pub suitability: Option<PrismaSuitabilityView>,
    /// True when the check read the abstract; `None` when there was no check.
    pub from_abstract: Option<bool>,
    /// The review variant the check named.
    pub variant: Option<String>,
    /// All 42 items, in checklist order.
    pub items: Vec<PrismaItemView>,
    /// A tally per section, in checklist order.
    pub sections: Vec<PrismaSectionView>,
    /// The key-item gap count.
    pub key_gaps: KeyGapsView,
    /// Every failed part.
    pub failed_parts: Vec<FailedPartView>,
    /// Every note the user should see: the check's, then each part's, in
    /// that order, each text once.
    pub notes: Vec<String>,
}

/// Everything the PRISMA tab opens with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrismaView {
    /// The paper's latest canonical text, the one a run goes to.
    pub canonical_id: i64,
    /// The newest run on that text, if there is one.
    pub current: Option<PrismaRunView>,
    /// Every other run, newest first.
    pub earlier: Vec<PrismaRunSummary>,
}

/// A condensation status worth showing: any but `completed`.
fn shown_status(status: Option<String>) -> Option<String> {
    status.filter(|s| s != "completed")
}

/// A check as shown.
fn suitability_view(check: &PrismaCheck) -> PrismaSuitabilityView {
    let s = &check.suitability;
    PrismaSuitabilityView {
        is_systematic_review: s.is_systematic_review,
        is_meta_analysis: s.is_meta_analysis,
        is_suitable: s.is_suitable,
        document_type: s.document_type.clone(),
        variant: s.variant.clone(),
        rationale: s.rationale.clone(),
        confidence: s.confidence,
        condensed_from_chars: s.condensed_from_chars,
        condensation_status: shown_status(s.condensation_status.clone()),
    }
}

/// Checklist item `row`'s view; `stored` is its shown row, if it has one.
/// `condensed_parts` holds the ids of the run's parts whose paper was
/// condensed.
fn item_view(
    row: &bmlib::quality::prisma2020_checklist::ChecklistItem,
    stored: Option<&StoredPrismaItem>,
    offsets: &[usize],
    condensed_parts: &HashSet<i64>,
) -> PrismaItemView {
    let mut view = PrismaItemView {
        id: row.id.to_string(),
        section: row.section.as_str().to_string(),
        name: row.name.to_string(),
        key: KEY_ITEMS.contains(&row.id),
        judgement: None,
        model_judgement: None,
        unbacked: false,
        condensed: false,
        rechecked: false,
        recheck_condensed: false,
        first_judgement: None,
        explanation: None,
        citations: vec![],
        quotes_not_found: vec![],
        quotes_contradicted: vec![],
        quotes_dropped: 0,
    };
    let Some(item) = stored else {
        return view;
    };
    let a = &item.assessment;
    view.judgement = Some(a.judgement.as_str().to_string());
    view.model_judgement = Some(a.model_judgement.as_str().to_string());
    view.unbacked = a.unbacked;
    view.condensed = item.condensed;
    view.rechecked = item.recheck_part_id.is_some();
    view.recheck_condensed = item
        .recheck_part_id
        .is_some_and(|id| condensed_parts.contains(&id));
    view.first_judgement = item.first_judgement.map(|j| j.as_str().to_string());
    view.explanation = Some(a.explanation.clone());
    view.citations = a
        .citations
        .iter()
        .map(|c| PrismaCitationView {
            span: span_to_utf16(
                offsets,
                TextSpan {
                    start: c.start,
                    end: c.end,
                },
            ),
            text: c.text.clone(),
            similarity: c.similarity,
            exact: c.exact,
        })
        .collect();
    view.quotes_not_found = a.quotes_not_found.clone();
    view.quotes_contradicted = a
        .quotes_contradicted
        .iter()
        .map(|q| ContradictedQuoteView {
            quote: q.quote.clone(),
            contradiction: q.contradiction.as_str().to_string(),
        })
        .collect();
    view.quotes_dropped = a.quotes_dropped;
    view
}

/// The key-gap count over `items` (all 42, in checklist order).
fn key_gaps(items: &[PrismaItemView], complete: bool) -> KeyGapsView {
    let shown = |id: &str| {
        items
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.judgement.as_deref())
    };
    let count = |wanted: &[&str]| {
        KEY_ITEMS
            .iter()
            .filter(|id| shown(id).is_some_and(|j| wanted.contains(&j)))
            .count()
    };
    KeyGapsView {
        items: KEY_ITEMS.iter().map(|id| (*id).to_string()).collect(),
        gaps: count(&[
            Judgement::NotReported.as_str(),
            Judgement::PartlyReported.as_str(),
        ]),
        elsewhere: count(&[Judgement::ReportedElsewhere.as_str()]),
        total: KEY_ITEMS.len(),
        complete,
    }
}

/// A tally per section of `items`, sections in order of first appearance.
fn sections(items: &[PrismaItemView]) -> Vec<PrismaSectionView> {
    let mut out: Vec<PrismaSectionView> = Vec::new();
    for item in items {
        if out.last().map(|s| s.section.as_str()) != Some(item.section.as_str()) {
            let counts = JUDGEMENTS
                .iter()
                .map(|j| j.as_str())
                .chain([NOT_ASSESSED])
                .map(|k| (k.to_string(), 0))
                .collect();
            out.push(PrismaSectionView {
                section: item.section.clone(),
                counts,
            });
        }
        let key = item.judgement.as_deref().unwrap_or(NOT_ASSESSED);
        if let Some(section) = out.last_mut() {
            *section.counts.entry(key.to_string()).or_insert(0) += 1;
        }
    }
    out
}

/// The stored PRISMA run `run_id`. Every citation's text is the paper's at its
/// span, and the spans are in the run's own text's UTF-16 units.
///
/// # Errors
///
/// [`StoreError::NotFound`] when there is no such run; a stored run this code
/// did not write (a part naming an item not on the checklist, a span outside
/// its text) as [`StoreError::Corrupt`]; any other store failure as
/// [`StoreError`].
pub fn prisma_run_view(db: &mut dyn Db, run_id: i64) -> Result<PrismaRunView, StoreError> {
    let run = stored_run(db, run_id)?
        .ok_or_else(|| StoreError::NotFound(format!("PRISMA run {run_id}")))?;
    let offsets = utf16_offsets(&run.text.text);
    let shown: HashMap<&str, &StoredPrismaItem> =
        run.items.iter().map(|i| (i.assessment.id, i)).collect();
    let condensed_parts: HashSet<i64> = run
        .parts
        .iter()
        .filter(|p| p.condensed_from_chars.is_some())
        .map(|p| p.id)
        .collect();
    let items: Vec<PrismaItemView> = CHECKLIST
        .iter()
        .map(|row| item_view(row, shown.get(row.id).copied(), &offsets, &condensed_parts))
        .collect();
    let mut failed_parts = Vec::new();
    for part in run.parts.iter().filter(|p| p.status == PART_FAILED) {
        let ids = part
            .item_ids
            .iter()
            .map(|id| {
                checklist_item(id).map(|i| i.id).ok_or_else(|| {
                    StoreError::Corrupt(format!(
                        "PRISMA run {run_id}: {id:?} is not a checklist item"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        failed_parts.push(FailedPartView {
            label: PartPlan { item_ids: ids }.label(),
            error_kind: part.error_kind.clone(),
            error_message: part.error_message.clone(),
        });
    }
    let mut notes: Vec<String> = Vec::new();
    let all = run
        .check
        .iter()
        .flat_map(|c| c.suitability.notes.iter())
        .chain(run.parts.iter().flat_map(|p| p.notes.iter()));
    for note in all {
        if !notes.contains(note) {
            notes.push(note.clone());
        }
    }
    let complete = run.status == RUN_COMPLETE;
    Ok(PrismaRunView {
        id: run_id,
        canonical_id: run.canonical_id,
        model: run.model,
        created_at: run.created_at,
        forced: run.forced,
        suitability: run.check.as_ref().map(suitability_view),
        from_abstract: run.check.as_ref().map(|c| c.from_abstract),
        variant: run.variant,
        sections: sections(&items),
        key_gaps: key_gaps(&items, complete),
        status: run.status,
        items,
        failed_parts,
        notes,
    })
}

/// The PRISMA tab of `paper`: its latest text, the newest run on it and every
/// other run. A run on an older text is listed in `earlier`, never current.
///
/// # Errors
///
/// [`StoreError::NotFound`] when the paper has no canonical text; any other
/// store failure as [`StoreError`].
pub fn prisma_view(
    db: &mut dyn Db,
    paper: &crate::store::PaperId,
) -> Result<PrismaView, StoreError> {
    let canonical_id = latest_canonical_id(db, paper)?
        .ok_or_else(|| StoreError::NotFound(format!("full text of paper {}", paper.as_str())))?;
    let current_id = latest_run(db, paper, canonical_id)?;
    let current = current_id.map(|id| prisma_run_view(db, id)).transpose()?;
    let earlier = runs(db, paper)?
        .into_iter()
        .filter(|r| Some(r.id) != current_id)
        .collect();
    Ok(PrismaView {
        canonical_id,
        current,
        earlier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind;
    use crate::prisma_store::{
        create_run, fail_part, finish_run, pending_parts, save_not_suitable, save_part,
    };
    use crate::store::{
        create_paper, open_memory_store, save_canonical, save_source, NewSource, PaperId,
        SourcePlace,
    };
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
    use bmlib::quality::prisma2020_models::Suitability;
    use bmlib::quality::{ItemAssessment, PartAssessment, PicoCitation};
    use rusqlite::Connection;

    /// A paper with astral characters before the cited block.
    const ASTRAL: &str = "# Trial 😀\n\n## Méthods 𝛼\n\nBody 😀 text.\n\nAfter.\n";

    /// Stores `md` as a new canonical text of `paper`.
    fn add_text(db: &mut Connection, paper: &PaperId, md: &str) -> i64 {
        let sid = save_source(
            db,
            paper,
            NewSource {
                kind: SourceKind::Markdown,
                origin: "paste".into(),
                file_name: None,
                bytes: md.as_bytes().to_vec(),
                decoded: None,
            },
            SourcePlace::App,
        )
        .unwrap();
        let text = canonical_from_markdown(md);
        let a = assess_full_text(&text).unwrap();
        save_canonical(db, paper, sid, &text, &a).unwrap()
    }

    /// A store with a paper on [`ASTRAL`].
    fn astral() -> (Connection, PaperId, i64) {
        let mut db = open_memory_store().unwrap();
        let p = create_paper(&mut db, &Identifiers::default(), Some("Trial")).unwrap();
        let cid = add_text(&mut db, &p, ASTRAL);
        (db, p, cid)
    }

    /// A passed check with a note.
    fn check(is_suitable: bool, notes: &[&str]) -> PrismaCheck {
        PrismaCheck {
            suitability: Suitability {
                is_systematic_review: is_suitable,
                is_meta_analysis: false,
                is_suitable,
                document_type: "systematic review".into(),
                variant: Some("scoping review".into()),
                rationale: "Because.".into(),
                confidence: 0.5,
                condensed_from_chars: None,
                condensation_status: Some("completed".into()),
                notes: notes.iter().map(|n| n.to_string()).collect(),
            },
            from_abstract: false,
        }
    }

    /// Item `id` shown `judgement`, with `citations`.
    fn assessed(
        id: &'static str,
        judgement: Judgement,
        cites: Vec<PicoCitation>,
    ) -> ItemAssessment {
        ItemAssessment {
            id,
            model_judgement: judgement,
            judgement,
            unbacked: false,
            explanation: format!("{id} explained"),
            citations: cites,
            quotes_not_found: vec![],
            quotes_contradicted: vec![],
            quotes_dropped: 0,
        }
    }

    /// `items` as one part's complete reply, with `notes`.
    fn reply(items: Vec<ItemAssessment>, notes: &[&str]) -> PartAssessment {
        PartAssessment {
            items,
            missing: vec![],
            condensed_from_chars: None,
            condensation_status: None,
            notes: notes.iter().map(|n| n.to_string()).collect(),
        }
    }

    /// A run with one part per entry of `parts`.
    fn new_run(db: &mut Connection, p: &PaperId, cid: i64, parts: &[&[&'static str]]) -> i64 {
        let plan: Vec<PartPlan> = parts
            .iter()
            .map(|ids| PartPlan {
                item_ids: ids.to_vec(),
            })
            .collect();
        create_run(
            db,
            p,
            cid,
            "m",
            Some(&check(true, &["check note"])),
            false,
            &plan,
        )
        .unwrap()
    }

    /// The run's pending part ids.
    fn pending(db: &mut Connection, run: i64) -> Vec<i64> {
        pending_parts(db, run)
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect()
    }

    /// There are always 42 items, in checklist order, whatever the run holds.
    #[test]
    fn items_are_42_in_checklist_order() {
        let (mut db, p, cid) = astral();
        let run = new_run(&mut db, &p, cid, &[&["1", "2"]]);
        let v = prisma_run_view(&mut db, run).unwrap();
        let ids: Vec<&str> = v.items.iter().map(|i| i.id.as_str()).collect();
        let want: Vec<&str> = CHECKLIST.iter().map(|i| i.id).collect();
        assert_eq!(ids, want);
        assert_eq!(ids.len(), 42);
        let one = &v.items[0];
        assert_eq!(
            (one.section.as_str(), one.name.as_str()),
            ("title", "Title")
        );
        assert_eq!(
            v.sections
                .iter()
                .map(|s| s.section.as_str())
                .collect::<Vec<_>>(),
            [
                "title",
                "abstract",
                "introduction",
                "methods",
                "results",
                "discussion",
                "other_information"
            ]
        );
    }

    /// Items with no shown row are Not assessed, and the tally says so.
    #[test]
    fn pending_items_are_not_assessed() {
        let (mut db, p, cid) = astral();
        let run = new_run(&mut db, &p, cid, &[&["1", "2"], &["3"]]);
        let first = pending(&mut db, run)[0];
        let a = reply(
            vec![
                assessed("1", Judgement::Reported, vec![]),
                assessed("2", Judgement::NotApplicable, vec![]),
            ],
            &[],
        );
        save_part(&mut db, first, &a, false).unwrap();
        let v = prisma_run_view(&mut db, run).unwrap();
        assert_eq!(v.items[0].judgement.as_deref(), Some("reported"));
        assert_eq!(v.items[2].judgement, None);
        assert_eq!(v.items[2].explanation, None);
        let title = &v.sections[0];
        assert_eq!(title.counts["reported"], 1);
        assert_eq!(title.counts[NOT_ASSESSED], 0);
        let intro = &v.sections[2];
        assert_eq!(intro.counts[NOT_ASSESSED], 2);
        assert_eq!(intro.counts["partly_reported"], 0, "zeros are kept");
        // A not-suitable run holds no part: all 42 are not assessed.
        let c = check(false, &[]);
        let ns = save_not_suitable(&mut db, &p, cid, "m", &c).unwrap();
        let v = prisma_run_view(&mut db, ns).unwrap();
        assert!(v.items.iter().all(|i| i.judgement.is_none()));
        assert_eq!(v.status, "complete");
        assert!(!v.suitability.unwrap().is_suitable);
    }

    /// Ten key items: 3 not reported, 1 unbacked, 1 partly, 2 elsewhere,
    /// 1 not applicable and 2 reported count 5 gaps and 2 elsewhere.
    #[test]
    fn key_gaps_count() {
        let (mut db, p, cid) = astral();
        let ids: [&'static str; 10] = KEY_ITEMS;
        let run = new_run(&mut db, &p, cid, &[&ids]);
        let part = pending(&mut db, run)[0];
        use Judgement::*;
        let shown = [
            NotReported,
            NotReported,
            NotReported,
            NotReported,
            PartlyReported,
            ReportedElsewhere,
            ReportedElsewhere,
            NotApplicable,
            Reported,
            Reported,
        ];
        let mut items: Vec<ItemAssessment> = ids
            .iter()
            .zip(shown)
            .map(|(id, j)| assessed(id, j, vec![]))
            .collect();
        items[3].model_judgement = Reported;
        items[3].unbacked = true;
        save_part(&mut db, part, &reply(items, &[]), false).unwrap();
        finish_run(&mut db, run).unwrap();
        let g = prisma_run_view(&mut db, run).unwrap().key_gaps;
        assert_eq!(g.items, KEY_ITEMS.map(String::from));
        assert_eq!((g.gaps, g.elsewhere, g.total), (5, 2, 10));
        assert!(g.complete);
        let v = prisma_run_view(&mut db, run).unwrap();
        let unbacked = v.items.iter().find(|i| i.id == ids[3]).unwrap();
        assert_eq!(unbacked.model_judgement.as_deref(), Some("reported"));
        assert!(unbacked.unbacked && unbacked.key);
        assert!(!v.items[0].key);
    }

    /// An unfinished run says so, and its failed part is named; notes are
    /// shown once each, the check's first.
    #[test]
    fn incomplete_run_is_not_complete() {
        let (mut db, p, cid) = astral();
        let run = new_run(&mut db, &p, cid, &[&["5", "6"], &["7"]]);
        let [a, b] = pending(&mut db, run)[..] else {
            panic!("not two parts")
        };
        let ok = reply(
            vec![
                assessed("5", Judgement::Reported, vec![]),
                assessed("6", Judgement::Reported, vec![]),
            ],
            &["check note", "part note"],
        );
        save_part(&mut db, a, &ok, false).unwrap();
        fail_part(&mut db, b, "timeout", "too slow").unwrap();
        let v = prisma_run_view(&mut db, run).unwrap();
        assert_eq!(v.status, "incomplete");
        assert!(!v.key_gaps.complete);
        assert_eq!(v.notes, ["check note", "part note"]);
        assert_eq!(v.failed_parts.len(), 1);
        assert_eq!(v.failed_parts[0].label, "Methods 7");
        assert_eq!(v.failed_parts[0].error_kind.as_deref(), Some("timeout"));
        assert_eq!(v.variant.as_deref(), Some("scoping review"));
        assert_eq!(v.from_abstract, Some(false));
    }

    /// A citation's UTF-16 span slices the paper's text (as JavaScript counts
    /// it) to exactly its `text`.
    #[test]
    fn spans_are_utf16() {
        let (mut db, p, cid) = astral();
        let t = crate::store::canonical_by_id(&mut db, &p, cid)
            .unwrap()
            .unwrap()
            .text;
        let needle = "Body 😀 text.";
        let byte = t.text.find(needle).unwrap();
        let start = t.text[..byte].chars().count();
        let end = start + needle.chars().count();
        let cite = PicoCitation {
            start,
            end,
            text: needle.into(),
            similarity: 0.9,
            exact: false,
        };
        let run = new_run(&mut db, &p, cid, &[&["1"]]);
        let part = pending(&mut db, run)[0];
        let a = reply(vec![assessed("1", Judgement::Reported, vec![cite])], &[]);
        save_part(&mut db, part, &a, false).unwrap();
        let c = &prisma_run_view(&mut db, run).unwrap().items[0].citations[0];
        let units: Vec<u16> = t.text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[c.span.start..c.span.end]).unwrap(),
            needle
        );
        assert!(
            c.span.end - c.span.start > needle.chars().count(),
            "astral units counted twice"
        );
        assert_eq!((c.similarity, c.exact), (0.9, false));
    }

    /// After a new text, the old run is listed as earlier and still computes
    /// its spans against its own text.
    #[test]
    fn earlier_run_uses_its_text() {
        let (mut db, p, old) = astral();
        let needle = "Body 😀 text.";
        let t = crate::store::canonical_by_id(&mut db, &p, old)
            .unwrap()
            .unwrap()
            .text;
        let byte = t.text.find(needle).unwrap();
        let start = t.text[..byte].chars().count();
        let cite = PicoCitation {
            start,
            end: start + needle.chars().count(),
            text: needle.into(),
            similarity: 1.0,
            exact: true,
        };
        let run = new_run(&mut db, &p, old, &[&["1"]]);
        let part = pending(&mut db, run)[0];
        let a = reply(vec![assessed("1", Judgement::Reported, vec![cite])], &[]);
        save_part(&mut db, part, &a, false).unwrap();
        let new = add_text(&mut db, &p, "# T\n\nAnother text.\n");
        let v = prisma_view(&mut db, &p).unwrap();
        assert_eq!(v.canonical_id, new);
        assert_eq!(v.current, None);
        assert_eq!(v.earlier.iter().map(|r| r.id).collect::<Vec<_>>(), [run]);
        let alone = prisma_run_view(&mut db, run).unwrap();
        assert_eq!(alone.canonical_id, old);
        // The span is in the OLD text's UTF-16 units, though a newer text exists.
        let c = &alone.items[0].citations[0];
        let units: Vec<u16> = t.text.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[c.span.start..c.span.end]).unwrap(),
            needle
        );
        let second = new_run(&mut db, &p, new, &[&["1"]]);
        let v = prisma_view(&mut db, &p).unwrap();
        assert_eq!(v.current.unwrap().id, second);
        assert_eq!(v.earlier.len(), 1);
    }

    /// A re-checked item says whether its re-check was condensed: from its
    /// re-check part's condensation, not the first answer's.
    #[test]
    fn recheck_says_if_condensed() {
        let (mut db, p, cid) = astral();
        let run = new_run(&mut db, &p, cid, &[&["7", "11"]]);
        let part = pending(&mut db, run)[0];
        let gaps = reply(
            vec![
                assessed("7", Judgement::NotReported, vec![]),
                assessed("11", Judgement::NotReported, vec![]),
            ],
            &[],
        );
        save_part(&mut db, part, &gaps, true).unwrap();
        let [seven, eleven] = pending(&mut db, run)[..] else {
            panic!("not two re-checks");
        };
        let mut condensed = reply(vec![assessed("7", Judgement::Reported, vec![])], &[]);
        condensed.condensed_from_chars = Some(300);
        save_part(&mut db, seven, &condensed, true).unwrap();
        let whole = reply(vec![assessed("11", Judgement::Reported, vec![])], &[]);
        save_part(&mut db, eleven, &whole, false).unwrap();
        let v = prisma_run_view(&mut db, run).unwrap();
        let get = |id: &str| v.items.iter().find(|i| i.id == id).unwrap();
        let flags = |i: &PrismaItemView| (i.condensed, i.rechecked, i.recheck_condensed);
        assert_eq!(flags(get("7")), (true, true, true));
        assert_eq!(flags(get("11")), (true, true, false));
        assert_eq!(flags(get("1")), (false, false, false));
    }

    /// An unknown run is not found.
    #[test]
    fn missing_run_is_not_found() {
        let (mut db, p, _) = astral();
        assert!(matches!(
            prisma_run_view(&mut db, 99),
            Err(StoreError::NotFound(_))
        ));
        let other = PaperId("nope".into());
        assert!(matches!(
            prisma_view(&mut db, &other),
            Err(StoreError::NotFound(_))
        ));
        let _ = p;
    }
}
