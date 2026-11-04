//! PICO runs on one paper: the suitability check, the extraction, and the
//! paper's spans backing each part. A run is on one canonical text, so every
//! stored span points into it.

use bmlib::db::{execute, fetch_all, fetch_one, transaction_with, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::{
    PicoCitation, PicoExtraction, PicoPart, PicoPartState, PicoSuitability, PICO_PARTS,
};
use serde::Serialize;

use crate::store::{canonical_by_id, last_id, PaperId, StoreError};

/// A suitability check and what it read.
#[derive(Debug, Clone, PartialEq)]
pub struct PicoCheck {
    /// The model's verdict.
    pub suitability: PicoSuitability,
    /// True when the check read the paper's abstract, false for the whole text.
    pub from_abstract: bool,
}

/// What a run found: the only two shapes a run has.
#[derive(Debug, Clone, PartialEq)]
// One run is held at a time, so the larger variant costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
pub enum PicoOutcome {
    /// The check found no PICO to extract; nothing else was asked.
    NotSuitable(PicoCheck),
    /// The extraction, after a passed check, or with none (`None`) when the
    /// run was forced.
    Extracted {
        /// The passed check, or `None` for a forced run.
        check: Option<PicoCheck>,
        /// The PICO parts and what backs them.
        extraction: PicoExtraction,
    },
}

/// A run to store.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPicoRun {
    /// The model that ran it.
    pub model: String,
    /// What it found.
    pub outcome: PicoOutcome,
}

/// A stored run: a [`NewPicoRun`] with its id, text and time.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPicoRun {
    /// The run's row id.
    pub id: i64,
    /// The canonical text it ran on.
    pub canonical_id: i64,
    /// The model that ran it.
    pub model: String,
    /// When it was stored (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// What it found; every citation's `text` re-read from the paper.
    pub outcome: PicoOutcome,
    /// The canonical text it ran on, as read to check its spans, so a view
    /// need not read it again (#58).
    pub text: CanonicalText,
}

/// One line of a paper's list of PICO runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PicoRunSummary {
    /// The run's row id.
    pub id: i64,
    /// The canonical text it ran on.
    pub canonical_id: i64,
    /// The model that ran it.
    pub model: String,
    /// When it was stored (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
}

/// The parts of `e` in [`PICO_PARTS`] order.
fn parts_of(e: &PicoExtraction) -> [&PicoPart; 4] {
    [&e.population, &e.intervention, &e.comparison, &e.outcome]
}

/// Stores `run` on canonical text `canonical_id` of `paper`, with its parts
/// and citations, in one transaction, and returns the run's id. An outcome
/// whose check contradicts its shape (an extraction after a failed check, or
/// "not suitable" from a passed one), a text that is not `paper`'s, or a
/// citation outside the text is [`StoreError::Corrupt`], never written: the
/// read side would refuse it ([`run`]).
pub fn save_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
    run: &NewPicoRun,
) -> Result<i64, StoreError> {
    let (check, extraction) = match &run.outcome {
        PicoOutcome::NotSuitable(c) if c.suitability.is_suitable => {
            return Err(StoreError::Corrupt(
                "a run that stopped at a passed check cannot be stored".into(),
            ))
        }
        PicoOutcome::NotSuitable(c) => (Some(c), None),
        PicoOutcome::Extracted { check: Some(c), .. } if !c.suitability.is_suitable => {
            return Err(StoreError::Corrupt(
                "an extraction after a failed check cannot be stored".into(),
            ))
        }
        PicoOutcome::Extracted { check, extraction } => (check.as_ref(), Some(extraction)),
    };
    let text = canonical_by_id(db, paper, canonical_id)?
        .ok_or_else(|| {
            StoreError::Corrupt(format!(
                "canonical text {canonical_id} is not paper {}'s",
                paper.as_str()
            ))
        })?
        .text;
    for c in extraction
        .iter()
        .flat_map(|e| parts_of(e))
        .flat_map(|p| &p.citations)
    {
        let span = TextSpan {
            start: c.start,
            end: c.end,
        };
        if text.span_text(span).is_none() {
            return Err(StoreError::Corrupt(format!(
                "citation {span} is outside canonical text {canonical_id}"
            )));
        }
    }
    let notes = extraction.map(|e| to_json(&e.notes)).transpose()?;
    transaction_with(db, |tx| {
        execute(
            tx,
            "INSERT INTO pico_runs (paper_id, canonical_id, model, suitability_json,
                                    suitability_from_abstract, extracted, study_type, sample_size,
                                    overall_confidence, condensed_from_chars, condensation_status,
                                    notes_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                Value::from(paper.as_str()),
                Value::from(canonical_id),
                Value::from(run.model.as_str()),
                Value::from(check.map(|c| c.suitability.to_json().to_string())),
                Value::from(check.map(|c| c.from_abstract)),
                Value::from(extraction.is_some()),
                Value::from(extraction.and_then(|e| e.study_type.clone())),
                Value::from(extraction.and_then(|e| e.sample_size.clone())),
                Value::from(extraction.and_then(|e| e.overall_confidence)),
                Value::from(extraction.and_then(|e| e.condensed_from_chars)),
                Value::from(extraction.and_then(|e| e.condensation_status.clone())),
                Value::from(notes.clone()),
            ],
        )?;
        let run_id = last_id(tx)?;
        for (name, part) in PICO_PARTS
            .iter()
            .zip(extraction.map(parts_of).into_iter().flatten())
        {
            execute(
                tx,
                "INSERT INTO pico_parts (run_id, part, description, state, confidence,
                                         quotes_not_found_json, quotes_dropped)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                &[
                    Value::from(run_id),
                    Value::from(*name),
                    Value::from(part.description.as_str()),
                    Value::from(part.state.as_str()),
                    Value::from(part.confidence),
                    Value::from(to_json(&part.quotes_not_found)?),
                    Value::from(part.quotes_dropped),
                ],
            )?;
            let part_id = last_id(tx)?;
            for (position, c) in part.citations.iter().enumerate() {
                execute(
                    tx,
                    "INSERT INTO pico_citations (part_id, position, span_start, span_end, similarity, exact)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    &[
                        Value::from(part_id),
                        Value::from(position),
                        Value::from(c.start),
                        Value::from(c.end),
                        Value::from(c.similarity),
                        Value::from(c.exact),
                    ],
                )?;
            }
        }
        Ok(run_id)
    })
}

/// `strings` as a JSON array.
fn to_json(strings: &[String]) -> Result<String, StoreError> {
    serde_json::to_string(strings)
        .map_err(|e| StoreError::Corrupt(format!("notes cannot be stored: {e}")))
}

/// The stored run `run_id`, or `None` if there is none. Every citation is
/// checked against the run's text and its `text` re-read from it. A row this
/// code did not write is [`StoreError::Corrupt`]: a missing text, a span
/// outside it, parts that are not the four in order, a check half stored, a
/// column of the wrong type, or a shape its check contradicts (an extraction
/// after a failed check, a stop at a passed one, a not-suitable run with parts).
pub fn run(db: &mut dyn Db, run_id: i64) -> Result<Option<StoredPicoRun>, StoreError> {
    let Some(r) = fetch_one(
        db,
        "SELECT paper_id, canonical_id, model, suitability_json, suitability_from_abstract,
                extracted, study_type, sample_size, overall_confidence, condensed_from_chars,
                condensation_status, notes_json, created_at
         FROM pico_runs WHERE id = ?",
        &[Value::from(run_id)],
    )?
    else {
        return Ok(None);
    };
    let bad = |why: &str| StoreError::Corrupt(format!("PICO run {run_id}: {why}"));
    let paper = PaperId(r.get_str("paper_id")?.to_string());
    let canonical_id = r.get_i64("canonical_id")?;
    let text = canonical_by_id(db, &paper, canonical_id)?
        .ok_or_else(|| bad("its canonical text is not stored"))?
        .text;
    let check = match (
        r.get("suitability_json")?.as_str(),
        r.get("suitability_from_abstract")?.as_i64(),
    ) {
        (Some(json), Some(from_abstract)) => Some(PicoCheck {
            suitability: suitability_from_json(json).map_err(|e| bad(&e))?,
            from_abstract: from_abstract != 0,
        }),
        (None, None) => None,
        _ => return Err(bad("its check is half stored")),
    };
    let outcome = if r.get_i64("extracted")? == 0 {
        let check = check.ok_or_else(|| bad("not suitable, but no check"))?;
        if check.suitability.is_suitable {
            return Err(bad("it stopped at a check that passed"));
        }
        let parts = fetch_one(
            db,
            "SELECT count(*) AS n FROM pico_parts WHERE run_id = ?",
            &[Value::from(run_id)],
        )?;
        if parts.map(|p| p.get_i64("n")).transpose()?.unwrap_or(0) > 0 {
            return Err(bad("not suitable, but it has parts"));
        }
        PicoOutcome::NotSuitable(check)
    } else {
        if check.as_ref().is_some_and(|c| !c.suitability.is_suitable) {
            return Err(bad("an extraction after a failed check"));
        }
        let notes = match r.get("notes_json")?.as_str() {
            Some(json) => from_json(json).map_err(|e| bad(&e))?,
            None => Vec::new(),
        };
        let [population, intervention, comparison, outcome] = parts_from(db, &text, run_id)?;
        PicoOutcome::Extracted {
            check,
            extraction: PicoExtraction {
                population,
                intervention,
                comparison,
                outcome,
                study_type: optional(&r, "study_type", text_of).map_err(|e| bad(&e))?,
                sample_size: optional(&r, "sample_size", text_of).map_err(|e| bad(&e))?,
                overall_confidence: optional(&r, "overall_confidence", Value::as_f64)
                    .map_err(|e| bad(&e))?,
                condensed_from_chars: optional(&r, "condensed_from_chars", Value::as_i64)
                    .map_err(|e| bad(&e))?,
                condensation_status: optional(&r, "condensation_status", text_of)
                    .map_err(|e| bad(&e))?,
                notes,
            },
        }
    };
    Ok(Some(StoredPicoRun {
        id: run_id,
        canonical_id,
        model: r.get_str("model")?.to_string(),
        created_at: r.get_str("created_at")?.to_string(),
        outcome,
        text,
    }))
}

/// Column `name` of `r` read by `read`, `None` when it is null. A value
/// `read` cannot read (a number where text belongs, say) is an error, never
/// "not given".
fn optional<T>(r: &Row, name: &str, read: fn(&Value) -> Option<T>) -> Result<Option<T>, String> {
    let v = r.get(name).map_err(|e| e.to_string())?;
    if v.is_null() {
        return Ok(None);
    }
    read(v)
        .map(Some)
        .ok_or_else(|| format!("its {name} is of the wrong type"))
}

/// `v` as owned text, if it is text.
fn text_of(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// A JSON array of strings read back.
fn from_json(json: &str) -> Result<Vec<String>, String> {
    serde_json::from_str(json).map_err(|e| format!("a list is invalid: {e}"))
}

/// A check stored as [`PicoSuitability::to_json`], read back field by field.
/// A field of the wrong type is an error; a missing or null one is `None`.
fn suitability_from_json(json: &str) -> Result<PicoSuitability, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("its check is not JSON: {e}"))?;
    let field = |name: &str| v.get(name).filter(|f| !f.is_null());
    let wrong = |name: &str| format!("its check's {name} is of the wrong type");
    let opt_bool = |name: &str| {
        field(name)
            .map(|f| f.as_bool().ok_or_else(|| wrong(name)))
            .transpose()
    };
    let opt_str = |name: &str| {
        field(name)
            .map(|f| f.as_str().map(str::to_string).ok_or_else(|| wrong(name)))
            .transpose()
    };
    let notes = match field("notes") {
        Some(n) => serde_json::from_value(n.clone()).map_err(|_| wrong("notes"))?,
        None => Vec::new(),
    };
    Ok(PicoSuitability {
        is_suitable: opt_bool("is_suitable")?.ok_or_else(|| wrong("is_suitable"))?,
        is_intervention_study: opt_bool("is_intervention_study")?,
        has_comparison: opt_bool("has_comparison")?,
        study_type: opt_str("study_type")?,
        confidence: field("confidence")
            .map(|f| f.as_f64().ok_or_else(|| wrong("confidence")))
            .transpose()?,
        rationale: opt_str("rationale")?,
        condensed_from_chars: field("condensed_from_chars")
            .map(|f| f.as_i64().ok_or_else(|| wrong("condensed_from_chars")))
            .transpose()?,
        condensation_status: opt_str("condensation_status")?,
        notes,
    })
}

/// The four parts of run `run_id` in [`PICO_PARTS`] order, each citation's
/// span checked against `text` and its wording read from it.
fn parts_from(
    db: &mut dyn Db,
    text: &CanonicalText,
    run_id: i64,
) -> Result<[PicoPart; 4], StoreError> {
    let bad = |why: String| StoreError::Corrupt(format!("PICO run {run_id}: {why}"));
    let rows = fetch_all(
        db,
        "SELECT id, part, description, state, confidence, quotes_not_found_json, quotes_dropped
         FROM pico_parts WHERE run_id = ? ORDER BY id",
        &[Value::from(run_id)],
    )?;
    let names: Vec<&str> = rows
        .iter()
        .map(|r| r.get_str("part"))
        .collect::<Result<_, _>>()?;
    if names != PICO_PARTS {
        return Err(bad(format!(
            "its parts are {names:?}, not the four in order"
        )));
    }
    let parts = rows
        .iter()
        .map(|r| part_from(db, text, r).map_err(bad))
        .collect::<Result<Vec<_>, _>>()?;
    parts
        .try_into()
        .map_err(|_| bad("its parts are not four".into()))
}

/// Every [`PicoPartState`], for reading one back by its stored string. The
/// test `part_states_are_all` breaks the build when bmlib adds a state.
const PART_STATES: [PicoPartState; 3] = [
    PicoPartState::Verified,
    PicoPartState::Unverified,
    PicoPartState::NotReported,
];

/// One part row and its citations.
fn part_from(db: &mut dyn Db, text: &CanonicalText, r: &Row) -> Result<PicoPart, String> {
    let name = r.get_str("part").map_err(|e| e.to_string())?;
    let state = r.get_str("state").map_err(|e| e.to_string())?;
    let part_id = r.get_i64("id").map_err(|e| e.to_string())?;
    let rows = fetch_all(
        db,
        "SELECT span_start, span_end, similarity, exact FROM pico_citations
         WHERE part_id = ? ORDER BY position",
        &[Value::from(part_id)],
    )
    .map_err(|e| e.to_string())?;
    let citations = rows
        .iter()
        .map(|c| citation_from(text, c).map_err(|e| format!("{name}: {e}")))
        .collect::<Result<_, _>>()?;
    let dropped = r.get_i64("quotes_dropped").map_err(|e| e.to_string())?;
    Ok(PicoPart {
        description: r
            .get_str("description")
            .map_err(|e| e.to_string())?
            .to_string(),
        state: PART_STATES
            .into_iter()
            .find(|s| s.as_str() == state)
            .ok_or_else(|| format!("{name}: unknown state {state:?}"))?,
        confidence: optional(r, "confidence", Value::as_f64).map_err(|e| format!("{name}: {e}"))?,
        citations,
        quotes_not_found: from_json(
            r.get_str("quotes_not_found_json")
                .map_err(|e| e.to_string())?,
        )?,
        quotes_dropped: usize::try_from(dropped)
            .map_err(|_| format!("{name}: negative dropped count"))?,
    })
}

/// One citation row, its wording the paper's.
fn citation_from(text: &CanonicalText, r: &Row) -> Result<PicoCitation, String> {
    let get = |n: &str| r.get_i64(n).map_err(|e| e.to_string());
    let to = |v: i64| usize::try_from(v).map_err(|_| "a span is negative".to_string());
    let span = TextSpan {
        start: to(get("span_start")?)?,
        end: to(get("span_end")?)?,
    };
    let wording = text
        .span_text(span)
        .ok_or_else(|| format!("span {span} is outside the text"))?;
    Ok(PicoCitation {
        start: span.start,
        end: span.end,
        text: wording.to_string(),
        similarity: r
            .get("similarity")
            .map_err(|e| e.to_string())?
            .as_f64()
            .ok_or("a similarity is not a number")?,
        exact: get("exact")? != 0,
    })
}

/// The paper and canonical text run `run_id` is on, or `None` if there is no
/// such run.
pub fn run_text(db: &mut dyn Db, run_id: i64) -> Result<Option<(PaperId, i64)>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT paper_id, canonical_id FROM pico_runs WHERE id = ?",
        &[Value::from(run_id)],
    )?;
    Ok(match row {
        Some(r) => Some((
            PaperId(r.get_str("paper_id")?.to_string()),
            r.get_i64("canonical_id")?,
        )),
        None => None,
    })
}

/// The newest run on canonical text `canonical_id` of `paper`.
pub fn latest_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<Option<i64>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT max(id) AS id FROM pico_runs WHERE paper_id = ? AND canonical_id = ?",
        &[Value::from(paper.as_str()), Value::from(canonical_id)],
    )?;
    Ok(match row {
        Some(r) => r.get("id")?.as_i64(),
        None => None,
    })
}

/// Every run on `paper`, newest first.
pub fn runs(db: &mut dyn Db, paper: &PaperId) -> Result<Vec<PicoRunSummary>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT id, canonical_id, model, created_at FROM pico_runs
         WHERE paper_id = ? ORDER BY id DESC",
        &[Value::from(paper.as_str())],
    )?;
    rows.iter()
        .map(|r| {
            Ok(PicoRunSummary {
                id: r.get_i64("id")?,
                canonical_id: r.get_i64("canonical_id")?,
                model: r.get_str("model")?.to_string(),
                created_at: r.get_str("created_at")?.to_string(),
            })
        })
        .collect()
}

/// Whether a run on canonical text `canonical_id` holds a citation: whether
/// spans point into that text, so it must not be re-derived or replaced. A run
/// with nothing cited (not suitable, or no part verified) is no dependent.
pub fn has_citations(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT 1 AS one FROM pico_citations c JOIN pico_parts p ON p.id = c.part_id
         JOIN pico_runs r ON r.id = p.run_id WHERE r.canonical_id = ? LIMIT 1",
        &[Value::from(canonical_id)],
    )?
    .is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind;
    use crate::store::{create_paper, open_memory_store, save_canonical, save_source, NewSource};
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_plain_text};
    use rusqlite::Connection;

    /// The paper's text: "Para one.\n\nPara two." as one plain-text paper.
    const TEXT: &str = "Para one.\n\nPara two.";

    /// A store with one paper and one canonical text; returns both ids.
    fn paper_with_text(c: &mut Connection) -> (PaperId, i64) {
        paper_with_pmid(c, "1")
    }

    /// Adds a paper with PMID `pmid` and one canonical text; returns both ids.
    fn paper_with_pmid(c: &mut Connection, pmid: &str) -> (PaperId, i64) {
        let ids = Identifiers {
            pmid: Some(pmid.into()),
            ..Default::default()
        };
        let p = create_paper(c, &ids, Some("T")).unwrap();
        let sid = save_source(
            c,
            &p,
            NewSource {
                kind: SourceKind::Markdown,
                origin: "paste".into(),
                file_name: None,
                bytes: TEXT.as_bytes().to_vec(),
                decoded: None,
            },
        )
        .unwrap();
        let text = canonical_from_plain_text(TEXT, "T");
        let a = assess_full_text(&text).unwrap();
        let cid = save_canonical(c, &p, sid, &text, &a).unwrap();
        (p, cid)
    }

    /// A citation of `quote` in the stored text, as bmlib gives it (the
    /// text is ASCII, so a byte offset is a code-point one).
    fn cite(c: &mut Connection, p: &PaperId, cid: i64, quote: &str) -> PicoCitation {
        let t = canonical_by_id(c, p, cid).unwrap().unwrap().text;
        let start = t.text.find(quote).unwrap();
        let end = start + quote.len();
        PicoCitation {
            start,
            end,
            text: t.span_text(TextSpan { start, end }).unwrap().to_string(),
            similarity: 1.0,
            exact: true,
        }
    }

    /// A part in `state` with `citations`.
    fn part(state: PicoPartState, citations: Vec<PicoCitation>) -> PicoPart {
        PicoPart {
            description: format!("{} part", state.as_str()),
            state,
            confidence: Some(0.5),
            citations,
            quotes_not_found: vec![],
            quotes_dropped: 0,
        }
    }

    /// A check with verdict `is_suitable`.
    fn check(is_suitable: bool, from_abstract: bool) -> PicoCheck {
        PicoCheck {
            suitability: PicoSuitability {
                is_suitable,
                is_intervention_study: Some(is_suitable),
                has_comparison: None,
                study_type: Some("RCT".into()),
                confidence: Some(0.75),
                rationale: Some("Because.".into()),
                condensed_from_chars: Some(1234),
                condensation_status: Some("completed".into()),
                notes: vec!["a note".into()],
            },
            from_abstract,
        }
    }

    /// An extraction: one verified part with two citations, one unverified
    /// with a not-found quote and a dropped one, two not reported.
    fn extraction(c: &mut Connection, p: &PaperId, cid: i64) -> PicoExtraction {
        let mut unverified = part(PicoPartState::Unverified, vec![]);
        unverified.quotes_not_found = vec!["Nowhere.".into()];
        unverified.quotes_dropped = 1;
        PicoExtraction {
            population: part(
                PicoPartState::Verified,
                vec![cite(c, p, cid, "Para two."), cite(c, p, cid, "Para")],
            ),
            intervention: unverified,
            comparison: part(PicoPartState::NotReported, vec![]),
            outcome: part(PicoPartState::NotReported, vec![]),
            study_type: Some("RCT".into()),
            sample_size: Some("105".into()),
            overall_confidence: Some(0.5),
            condensed_from_chars: None,
            condensation_status: None,
            notes: vec!["n1".into(), "n2".into()],
        }
    }

    /// Stores `outcome` with model "m" and returns the run's id.
    fn store(c: &mut Connection, p: &PaperId, cid: i64, outcome: PicoOutcome) -> i64 {
        let run = NewPicoRun {
            model: "m".into(),
            outcome,
        };
        save_run(c, p, cid, &run).unwrap()
    }

    /// A run with a check and an extraction reads back as stored, citations
    /// in order and their text the paper's, whichever the check read.
    #[test]
    fn run_round_trip() {
        for from_abstract in [true, false] {
            let mut c = open_memory_store().unwrap();
            let (p, cid) = paper_with_text(&mut c);
            let outcome = PicoOutcome::Extracted {
                check: Some(check(true, from_abstract)),
                extraction: extraction(&mut c, &p, cid),
            };
            let id = store(&mut c, &p, cid, outcome.clone());
            let got = run(&mut c, id).unwrap().unwrap();
            assert_eq!((got.id, got.canonical_id), (id, cid));
            assert_eq!(got.model, "m");
            assert!(!got.created_at.is_empty());
            assert_eq!(got.outcome, outcome);
            assert_eq!(
                got.text,
                canonical_by_id(&mut c, &p, cid).unwrap().unwrap().text
            );
            let PicoOutcome::Extracted { extraction: e, .. } = got.outcome else {
                panic!("not extracted");
            };
            assert_eq!(e.population.citations[0].text, "Para two.");
            assert_eq!(e.population.citations[1].text, "Para");
            assert_eq!(run(&mut c, id + 1).unwrap(), None);
            assert_eq!(run_text(&mut c, id).unwrap(), Some((p.clone(), cid)));
            assert_eq!(run_text(&mut c, id + 1).unwrap(), None);
        }
    }

    /// A forced run has no check.
    #[test]
    fn forced_run_has_no_check() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let outcome = PicoOutcome::Extracted {
            check: None,
            extraction: extraction(&mut c, &p, cid),
        };
        let id = store(&mut c, &p, cid, outcome.clone());
        assert_eq!(run(&mut c, id).unwrap().unwrap().outcome, outcome);
    }

    /// A run that stopped at the check stores no parts.
    #[test]
    fn not_suitable_run_has_no_parts() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let outcome = PicoOutcome::NotSuitable(check(false, true));
        let id = store(&mut c, &p, cid, outcome.clone());
        assert_eq!(run(&mut c, id).unwrap().unwrap().outcome, outcome);
        assert_eq!(count(&mut c, "pico_parts"), 0);
    }

    /// An outcome its check contradicts is refused and nothing is written.
    #[test]
    fn mismatched_outcome_refused() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let bad = [
            PicoOutcome::Extracted {
                check: Some(check(false, true)),
                extraction: extraction(&mut c, &p, cid),
            },
            PicoOutcome::NotSuitable(check(true, true)),
        ];
        for outcome in bad {
            let run = NewPicoRun {
                model: "m".into(),
                outcome,
            };
            assert!(matches!(
                save_run(&mut c, &p, cid, &run),
                Err(StoreError::Corrupt(_))
            ));
        }
        assert_eq!(count(&mut c, "pico_runs"), 0);
    }

    /// The latest run is the newest on that text; the list holds every run,
    /// newest first.
    #[test]
    fn latest_is_newest_on_that_text() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let first = store(
            &mut c,
            &p,
            cid,
            PicoOutcome::NotSuitable(check(false, true)),
        );
        let second = store(
            &mut c,
            &p,
            cid,
            PicoOutcome::NotSuitable(check(false, true)),
        );
        execute(
            &mut c,
            "INSERT INTO canonical_texts (paper_id, source_id, converter_version, canonical_json, assessment_json)
             SELECT paper_id, source_id, converter_version, canonical_json, assessment_json FROM canonical_texts",
            &[],
        )
        .unwrap();
        let third = store(
            &mut c,
            &p,
            cid + 1,
            PicoOutcome::NotSuitable(check(false, true)),
        );
        assert_eq!(latest_run(&mut c, &p, cid).unwrap(), Some(second));
        assert_eq!(latest_run(&mut c, &p, cid + 1).unwrap(), Some(third));
        assert_eq!(latest_run(&mut c, &p, cid + 2).unwrap(), None);
        let list = runs(&mut c, &p).unwrap();
        let ids: Vec<i64> = list.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![third, second, first]);
        assert_eq!(list[0].canonical_id, cid + 1);
        assert_eq!(list[0].model, "m");
        assert!(!list[0].created_at.is_empty());
    }

    /// Rows this code did not write are reported, not shown.
    #[test]
    fn corrupt_rows_reported() {
        let cases: &[(&str, &[&str])] = &[
            (
                "span past the end",
                &["UPDATE pico_citations SET span_end = 9999 WHERE position = 0"],
            ),
            (
                "start after end",
                &["UPDATE pico_citations SET span_start = 5, span_end = 2 WHERE position = 0"],
            ),
            (
                "three parts",
                &["DELETE FROM pico_parts WHERE part = 'outcome'"],
            ),
            (
                "two parts swapped",
                &[
                    "UPDATE pico_parts SET part = 'x' WHERE part = 'comparison'",
                    "UPDATE pico_parts SET part = 'comparison' WHERE part = 'outcome'",
                    "UPDATE pico_parts SET part = 'outcome' WHERE part = 'x'",
                ],
            ),
            ("extracted with no parts", &["DELETE FROM pico_parts"]),
            (
                "extracted after a failed check",
                &["UPDATE pico_runs SET suitability_json = replace(suitability_json, '\"is_suitable\":true', '\"is_suitable\":false')"],
            ),
            (
                "not suitable after a passed check",
                &[
                    "DELETE FROM pico_parts",
                    "UPDATE pico_runs SET extracted = 0",
                ],
            ),
            (
                "not suitable with parts",
                &["UPDATE pico_runs SET extracted = 0, suitability_json = replace(suitability_json, '\"is_suitable\":true', '\"is_suitable\":false')"],
            ),
            (
                "unknown state",
                &["UPDATE pico_parts SET state = 'weird' WHERE part = 'outcome'"],
            ),
            (
                "not suitable with no check",
                &[
                    "DELETE FROM pico_parts",
                    "UPDATE pico_runs SET extracted = 0, suitability_json = NULL, suitability_from_abstract = NULL",
                ],
            ),
            (
                "check without what it read",
                &["UPDATE pico_runs SET suitability_from_abstract = NULL"],
            ),
            (
                "what it read without a check",
                &["UPDATE pico_runs SET suitability_json = NULL"],
            ),
            (
                "check field of the wrong type",
                &["UPDATE pico_runs SET suitability_json = replace(suitability_json, '\"is_suitable\":true', '\"is_suitable\":\"yes\"')"],
            ),
            ("notes not a list", &["UPDATE pico_runs SET notes_json = '{'"]),
            ("study type not text", &["UPDATE pico_runs SET study_type = X'05'"]),
            (
                "confidence not a number",
                &["UPDATE pico_runs SET overall_confidence = 'high'"],
            ),
            (
                "part confidence not a number",
                &["UPDATE pico_parts SET confidence = 'high' WHERE part = 'outcome'"],
            ),
            (
                "negative span",
                &["UPDATE pico_citations SET span_start = -1 WHERE position = 0"],
            ),
            (
                "negative dropped count",
                &["UPDATE pico_parts SET quotes_dropped = -1 WHERE part = 'outcome'"],
            ),
        ];
        for &(what, sql) in cases {
            let mut c = open_memory_store().unwrap();
            let (p, cid) = paper_with_text(&mut c);
            let outcome = PicoOutcome::Extracted {
                check: Some(check(true, true)),
                extraction: extraction(&mut c, &p, cid),
            };
            let id = store(&mut c, &p, cid, outcome);
            for s in sql {
                execute(&mut c, s, &[]).unwrap();
            }
            assert!(
                matches!(run(&mut c, id), Err(StoreError::Corrupt(_))),
                "{what}"
            );
        }
    }

    /// A run whose row points at another paper's text is reported, never read
    /// (or shown) with that text.
    #[test]
    fn run_on_another_papers_text_reported() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let outcome = PicoOutcome::Extracted {
            check: None,
            extraction: extraction(&mut c, &p, cid),
        };
        let id = store(&mut c, &p, cid, outcome);
        let (_, other) = paper_with_pmid(&mut c, "2");
        execute(
            &mut c,
            "UPDATE pico_runs SET canonical_id = ? WHERE id = ?",
            &[Value::from(other), Value::from(id)],
        )
        .unwrap();
        assert!(matches!(run(&mut c, id), Err(StoreError::Corrupt(_))));
        assert!(matches!(
            crate::view::pico_run_view(&mut c, id),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A citation outside the text, or a text that is not the paper's, is
    /// refused and nothing is written.
    #[test]
    fn save_checks_text_and_spans() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let mut outside = extraction(&mut c, &p, cid);
        outside.population.citations[0].end = 9999;
        let forced = |extraction| NewPicoRun {
            model: "m".into(),
            outcome: PicoOutcome::Extracted {
                check: None,
                extraction,
            },
        };
        assert!(matches!(
            save_run(&mut c, &p, cid, &forced(outside)),
            Err(StoreError::Corrupt(_))
        ));
        let other = PaperId("no-such-paper".into());
        let fine = extraction(&mut c, &p, cid);
        assert!(matches!(
            save_run(&mut c, &other, cid, &forced(fine.clone())),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            save_run(&mut c, &p, cid + 1, &forced(fine)),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(count(&mut c, "pico_runs"), 0);
    }

    /// A save that fails part-way writes nothing: no run without its parts.
    #[test]
    fn save_is_one_transaction() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let outcome = PicoOutcome::Extracted {
            check: None,
            extraction: extraction(&mut c, &p, cid),
        };
        execute(&mut c, "DROP TABLE pico_citations", &[]).unwrap();
        let run = NewPicoRun {
            model: "m".into(),
            outcome,
        };
        assert!(save_run(&mut c, &p, cid, &run).is_err());
        assert_eq!(count(&mut c, "pico_runs"), 0);
        assert_eq!(count(&mut c, "pico_parts"), 0);
    }

    /// [`PART_STATES`] holds every state: a state bmlib adds fails this match
    /// to compile, and every listed one reads back.
    #[test]
    fn part_states_are_all() {
        for s in PART_STATES {
            match s {
                PicoPartState::Verified
                | PicoPartState::Unverified
                | PicoPartState::NotReported => {}
            }
        }
        let names: Vec<&str> = PART_STATES.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, ["verified", "unverified", "not_reported"]);
    }

    /// Only a stored citation makes a text a dependent.
    #[test]
    fn has_citations_counts_only_citations() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        store(
            &mut c,
            &p,
            cid,
            PicoOutcome::NotSuitable(check(false, true)),
        );
        assert!(!has_citations(&mut c, cid).unwrap());
        let mut uncited = extraction(&mut c, &p, cid);
        uncited.population = part(PicoPartState::Unverified, vec![]);
        let forced = |extraction| PicoOutcome::Extracted {
            check: None,
            extraction,
        };
        store(&mut c, &p, cid, forced(uncited));
        assert!(!has_citations(&mut c, cid).unwrap());
        let cited = extraction(&mut c, &p, cid);
        store(&mut c, &p, cid, forced(cited));
        assert!(has_citations(&mut c, cid).unwrap());
        assert!(!has_citations(&mut c, cid + 1).unwrap());
    }

    /// The number of rows in `table`.
    fn count(c: &mut Connection, table: &str) -> i64 {
        fetch_one(c, &format!("SELECT count(*) AS n FROM {table}"), &[])
            .unwrap()
            .unwrap()
            .get_i64("n")
            .unwrap()
    }
}
