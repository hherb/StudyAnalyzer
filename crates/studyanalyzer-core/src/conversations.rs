//! Conversations with one paper: turns, and the verified quotes each answer
//! cites. A conversation is on one canonical text, so every stored span points
//! into it.

use bmlib::db::{execute, fetch_all, fetch_one, transaction_with, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use serde::Serialize;

use crate::store::{last_id, PaperId, StoreError};

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// The answer finished.
    Complete,
    /// The user stopped it; the answer is what had arrived.
    Stopped,
    /// The model call failed; `error` says why.
    Failed,
}

impl TurnStatus {
    /// The text stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            TurnStatus::Complete => "complete",
            TurnStatus::Stopped => "stopped",
            TurnStatus::Failed => "failed",
        }
    }

    /// The status stored as `s`, or `None` for an unknown text.
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Complete, Self::Stopped, Self::Failed]
            .into_iter()
            .find(|v| v.as_str() == s)
    }
}

/// What the model was given of the paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The whole canonical text.
    WholePaper,
    /// Only the passages retrieved for the question.
    Retrieval,
}

impl Mode {
    /// The text stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::WholePaper => "whole_paper",
            Mode::Retrieval => "retrieval",
        }
    }

    /// The mode stored as `s`, or `None` for an unknown text.
    pub fn parse(s: &str) -> Option<Self> {
        [Self::WholePaper, Self::Retrieval]
            .into_iter()
            .find(|v| v.as_str() == s)
    }
}

/// One quote an answer cited, as found by the passage matcher.
#[derive(Debug, Clone, PartialEq)]
pub struct NewQuote {
    /// The citation number shown in the answer; `Some` exactly when located.
    pub number: Option<u32>,
    /// The quote as the model wrote it.
    pub quote: String,
    /// The paper's own span for it; `None` when it could not be located.
    pub span: Option<TextSpan>,
    /// How closely the model's copy matched the paper's span.
    pub similarity: f64,
}

/// A turn to store.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTurn {
    /// The user's question.
    pub question: String,
    /// The answer, whole or as far as it got.
    pub answer: String,
    /// How the turn ended.
    pub status: TurnStatus,
    /// Why it failed for a failed turn, or the cut-off notice for a complete
    /// one. Read `status`, not this, to tell them apart.
    pub error: Option<String>,
    /// The model that answered.
    pub model: String,
    /// What the model was given of the paper.
    pub mode: Mode,
    /// The retrieved passages (empty for the whole paper).
    pub passages: Vec<TextSpan>,
    /// The quotes the answer cited, in order.
    pub quotes: Vec<NewQuote>,
    /// Tokens sent to the model.
    pub input_tokens: i64,
    /// Tokens the model returned.
    pub output_tokens: i64,
}

/// A stored turn: a [`NewTurn`] with its id and time.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTurn {
    /// The turn's row id.
    pub id: i64,
    /// The user's question.
    pub question: String,
    /// The answer, whole or as far as it got.
    pub answer: String,
    /// How the turn ended.
    pub status: TurnStatus,
    /// Why it failed for a failed turn, or the cut-off notice for a complete
    /// one. Read `status`, not this, to tell them apart.
    pub error: Option<String>,
    /// The model that answered.
    pub model: String,
    /// What the model was given of the paper.
    pub mode: Mode,
    /// The retrieved passages (empty for the whole paper).
    pub passages: Vec<TextSpan>,
    /// The quotes the answer cited, in order.
    pub quotes: Vec<NewQuote>,
    /// Tokens sent to the model.
    pub input_tokens: i64,
    /// Tokens the model returned.
    pub output_tokens: i64,
    /// When the turn was stored (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
}

/// One line of a paper's list of conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationSummary {
    /// The conversation's row id.
    pub id: i64,
    /// The canonical text it is on.
    pub canonical_id: i64,
    /// How many turns it holds.
    pub turns: usize,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
}

/// Starts an empty conversation with `paper` on canonical text `canonical_id`
/// and returns its id.
pub fn create_conversation(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<i64, StoreError> {
    execute(
        db,
        "INSERT INTO conversations (paper_id, canonical_id) VALUES (?, ?)",
        &[Value::from(paper.as_str()), Value::from(canonical_id)],
    )?;
    last_id(db)
}

/// The newest conversation with `paper` on canonical text `canonical_id`.
pub fn current_conversation(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<Option<i64>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT max(id) AS id FROM conversations WHERE paper_id = ? AND canonical_id = ?",
        &[Value::from(paper.as_str()), Value::from(canonical_id)],
    )?;
    Ok(match row {
        Some(r) => r.get("id")?.as_i64(),
        None => None,
    })
}

/// Every conversation with `paper`, newest first.
pub fn conversations(
    db: &mut dyn Db,
    paper: &PaperId,
) -> Result<Vec<ConversationSummary>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT c.id, c.canonical_id, c.created_at,
                (SELECT count(*) FROM turns t WHERE t.conversation_id = c.id) AS turns
         FROM conversations c WHERE c.paper_id = ? ORDER BY c.id DESC",
        &[Value::from(paper.as_str())],
    )?;
    rows.iter()
        .map(|r| {
            Ok(ConversationSummary {
                id: r.get_i64("id")?,
                canonical_id: r.get_i64("canonical_id")?,
                turns: usize::try_from(r.get_i64("turns")?)
                    .map_err(|_| StoreError::Corrupt("negative turn count".into()))?,
                created_at: r.get_str("created_at")?.to_string(),
            })
        })
        .collect()
}

/// The paper and canonical text a conversation is on, or `None` if there is
/// no such conversation.
pub fn conversation_text(
    db: &mut dyn Db,
    conversation_id: i64,
) -> Result<Option<(PaperId, i64)>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT paper_id, canonical_id FROM conversations WHERE id = ?",
        &[Value::from(conversation_id)],
    )?;
    Ok(match row {
        Some(r) => Some((
            PaperId(r.get_str("paper_id")?.to_string()),
            r.get_i64("canonical_id")?,
        )),
        None => None,
    })
}

/// Stores `turn` and its quotes in one transaction and returns the turn's id.
/// [`StoreError::NotFound`] when the conversation does not exist.
pub fn save_turn(db: &mut dyn Db, conversation_id: i64, turn: &NewTurn) -> Result<i64, StoreError> {
    let passages = serde_json::to_string(&turn.passages)
        .map_err(|e| StoreError::Corrupt(format!("passages cannot be stored: {e}")))?;
    transaction_with(db, |tx| {
        if conversation_text(tx, conversation_id)?.is_none() {
            return Err(StoreError::NotFound(format!(
                "conversation {conversation_id}"
            )));
        }
        execute(
            tx,
            "INSERT INTO turns (conversation_id, question, answer, status, error, model, mode,
                                passages_json, input_tokens, output_tokens)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                Value::from(conversation_id),
                Value::from(turn.question.as_str()),
                Value::from(turn.answer.as_str()),
                Value::from(turn.status.as_str()),
                Value::from(turn.error.clone()),
                Value::from(turn.model.as_str()),
                Value::from(turn.mode.as_str()),
                Value::from(passages),
                Value::from(turn.input_tokens),
                Value::from(turn.output_tokens),
            ],
        )?;
        let turn_id = last_id(tx)?;
        for (position, q) in turn.quotes.iter().enumerate() {
            execute(
                tx,
                "INSERT INTO quotes (turn_id, position, number, quote, span_start, span_end, similarity)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                &[
                    Value::from(turn_id),
                    Value::from(position),
                    Value::from(q.number.map(i64::from)),
                    Value::from(q.quote.as_str()),
                    Value::from(q.span.map(|s| s.start as i64)),
                    Value::from(q.span.map(|s| s.end as i64)),
                    Value::from(q.similarity),
                ],
            )?;
        }
        Ok(turn_id)
    })
}

/// The conversation's turns, in the order they were stored. Every span read
/// back is checked against the conversation's canonical text; one outside it
/// (or a number without a span) is [`StoreError::Corrupt`].
pub fn turns(db: &mut dyn Db, conversation_id: i64) -> Result<Vec<StoredTurn>, StoreError> {
    let Some(text_row) = fetch_one(
        db,
        "SELECT t.canonical_json FROM conversations c
         JOIN canonical_texts t ON t.id = c.canonical_id WHERE c.id = ?",
        &[Value::from(conversation_id)],
    )?
    else {
        return Ok(Vec::new());
    };
    let value: serde_json::Value = serde_json::from_str(text_row.get_str("canonical_json")?)
        .map_err(|e| StoreError::Corrupt(format!("canonical text is not JSON: {e}")))?;
    let text = CanonicalText::from_value(value)
        .map_err(|e| StoreError::Corrupt(format!("canonical text is invalid: {e}")))?;
    let rows = fetch_all(
        db,
        "SELECT id, question, answer, status, error, model, mode, passages_json,
                input_tokens, output_tokens, created_at
         FROM turns WHERE conversation_id = ? ORDER BY id",
        &[Value::from(conversation_id)],
    )?;
    rows.iter().map(|r| turn_from_row(db, &text, r)).collect()
}

/// Reads one turn row and its quotes, checking every span against `text`.
fn turn_from_row(db: &mut dyn Db, text: &CanonicalText, r: &Row) -> Result<StoredTurn, StoreError> {
    let id = r.get_i64("id")?;
    let corrupt =
        |what: &str, v: &str| StoreError::Corrupt(format!("turn {id}: unknown {what} {v:?}"));
    let status = r.get_str("status")?;
    let mode = r.get_str("mode")?;
    let passages: Vec<TextSpan> = serde_json::from_str(r.get_str("passages_json")?)
        .map_err(|e| StoreError::Corrupt(format!("turn {id}: passages are invalid: {e}")))?;
    if let Some(bad) = passages.iter().find(|s| text.span_text(**s).is_none()) {
        return Err(StoreError::Corrupt(format!(
            "turn {id}: passage {bad} is outside the text"
        )));
    }
    Ok(StoredTurn {
        id,
        question: r.get_str("question")?.to_string(),
        answer: r.get_str("answer")?.to_string(),
        status: TurnStatus::parse(status).ok_or_else(|| corrupt("status", status))?,
        error: r.get("error")?.as_str().map(str::to_string),
        model: r.get_str("model")?.to_string(),
        mode: Mode::parse(mode).ok_or_else(|| corrupt("mode", mode))?,
        passages,
        quotes: quotes_of(db, text, id)?,
        input_tokens: r.get_i64("input_tokens")?,
        output_tokens: r.get_i64("output_tokens")?,
        created_at: r.get_str("created_at")?.to_string(),
    })
}

/// The quotes of turn `turn_id` in order, each span checked against `text`.
fn quotes_of(
    db: &mut dyn Db,
    text: &CanonicalText,
    turn_id: i64,
) -> Result<Vec<NewQuote>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT number, quote, span_start, span_end, similarity
         FROM quotes WHERE turn_id = ? ORDER BY position",
        &[Value::from(turn_id)],
    )?;
    rows.iter()
        .map(|r| {
            let bad = |why: &str| StoreError::Corrupt(format!("turn {turn_id}: quote {why}"));
            let number = match r.get("number")?.as_i64() {
                Some(n) => Some(u32::try_from(n).map_err(|_| bad("number is out of range"))?),
                None => None,
            };
            let span = match (r.get("span_start")?.as_i64(), r.get("span_end")?.as_i64()) {
                (Some(s), Some(e)) => {
                    let to = |v: i64| usize::try_from(v).map_err(|_| bad("span is negative"));
                    let span = TextSpan {
                        start: to(s)?,
                        end: to(e)?,
                    };
                    if text.span_text(span).is_none() {
                        return Err(bad(&format!("span {span} is outside the text")));
                    }
                    Some(span)
                }
                (None, None) => None,
                _ => return Err(bad("span is half stored")),
            };
            if number.is_some() != span.is_some() {
                return Err(bad("number and span are not stored together"));
            }
            Ok(NewQuote {
                number,
                quote: r.get_str("quote")?.to_string(),
                span,
                similarity: r
                    .get("similarity")?
                    .as_f64()
                    .ok_or_else(|| bad("similarity is not a number"))?,
            })
        })
        .collect()
}

/// Whether any conversation on canonical text `canonical_id` holds a turn:
/// whether spans point into that text, so it must not be re-derived or replaced.
pub fn has_turns(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT 1 AS one FROM turns t JOIN conversations c ON c.id = t.conversation_id
         WHERE c.canonical_id = ? LIMIT 1",
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

    /// A store with one paper and one canonical text; returns both ids.
    fn paper_with_text(c: &mut Connection) -> (PaperId, i64) {
        let ids = Identifiers {
            pmid: Some("1".into()),
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
                bytes: b"Para one.\n\nPara two.".to_vec(),
                decoded: None,
            },
        )
        .unwrap();
        let text = canonical_from_plain_text("Para one.\n\nPara two.", "T");
        let a = assess_full_text(&text).unwrap();
        let cid = save_canonical(c, &p, sid, &text, &a).unwrap();
        (p, cid)
    }

    /// A turn with a located and an unlocated quote.
    fn a_turn() -> NewTurn {
        NewTurn {
            question: "Q?".into(),
            answer: "A [1] and B.".into(),
            status: TurnStatus::Complete,
            error: None,
            model: "m".into(),
            mode: Mode::Retrieval,
            passages: vec![TextSpan { start: 0, end: 4 }],
            quotes: vec![
                NewQuote {
                    number: Some(1),
                    quote: "Para".into(),
                    span: Some(TextSpan { start: 0, end: 4 }),
                    similarity: 1.0,
                },
                NewQuote {
                    number: None,
                    quote: "Nowhere".into(),
                    span: None,
                    similarity: 0.25,
                },
            ],
            input_tokens: 10,
            output_tokens: 3,
        }
    }

    /// A turn reads back as stored, quotes in order.
    #[test]
    fn turn_round_trip() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let conv = create_conversation(&mut c, &p, cid).unwrap();
        let t = a_turn();
        let id = save_turn(&mut c, conv, &t).unwrap();
        let got = turns(&mut c, conv).unwrap();
        assert_eq!(got.len(), 1);
        let g = got[0].clone();
        assert_eq!(g.id, id);
        assert_eq!(g.quotes, t.quotes);
        assert_eq!(g.passages, t.passages);
        assert_eq!(
            (g.question, g.answer, g.status),
            (t.question, t.answer, t.status)
        );
        assert_eq!((g.model, g.mode, g.error), (t.model, t.mode, t.error));
        assert_eq!((g.input_tokens, g.output_tokens), (10, 3));
        assert!(!g.created_at.is_empty());
        assert_eq!(conversation_text(&mut c, conv).unwrap(), Some((p, cid)));
        assert_eq!(conversation_text(&mut c, conv + 1).unwrap(), None);
    }

    /// The current conversation is the newest on that text; the list is newest first.
    #[test]
    fn current_is_newest_on_that_text() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let first = create_conversation(&mut c, &p, cid).unwrap();
        let second = create_conversation(&mut c, &p, cid).unwrap();
        // A second canonical text, a copy of the first.
        execute(
            &mut c,
            "INSERT INTO canonical_texts (paper_id, source_id, converter_version, canonical_json, assessment_json)
             SELECT paper_id, source_id, converter_version, canonical_json, assessment_json FROM canonical_texts",
            &[],
        )
        .unwrap();
        let third = create_conversation(&mut c, &p, cid + 1).unwrap();
        assert_eq!(current_conversation(&mut c, &p, cid).unwrap(), Some(second));
        assert_eq!(
            current_conversation(&mut c, &p, cid + 1).unwrap(),
            Some(third)
        );
        assert_eq!(current_conversation(&mut c, &p, cid + 2).unwrap(), None);
        save_turn(&mut c, first, &a_turn()).unwrap();
        let list = conversations(&mut c, &p).unwrap();
        let ids: Vec<i64> = list.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![third, second, first]);
        assert_eq!(list[2].turns, 1);
        assert_eq!(list[0].turns, 0);
    }

    /// An empty conversation does not keep its text; a saved turn does.
    #[test]
    fn empty_conversation_is_no_dependent() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let conv = create_conversation(&mut c, &p, cid).unwrap();
        assert!(!has_turns(&mut c, cid).unwrap());
        save_turn(&mut c, conv, &a_turn()).unwrap();
        assert!(has_turns(&mut c, cid).unwrap());
        assert!(!has_turns(&mut c, cid + 1).unwrap());
    }

    /// A turn for a missing conversation is refused.
    #[test]
    fn turn_for_missing_conversation_is_not_found() {
        let mut c = open_memory_store().unwrap();
        assert!(matches!(
            save_turn(&mut c, 7, &a_turn()),
            Err(StoreError::NotFound(_))
        ));
    }

    /// A stored span past the text, a number without a span, or an unknown
    /// status is reported, not shown.
    #[test]
    fn corrupt_span_reported() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let conv = create_conversation(&mut c, &p, cid).unwrap();
        save_turn(&mut c, conv, &a_turn()).unwrap();
        execute(
            &mut c,
            "UPDATE quotes SET span_end = 9999 WHERE number = 1",
            &[],
        )
        .unwrap();
        assert!(matches!(turns(&mut c, conv), Err(StoreError::Corrupt(_))));
        execute(
            &mut c,
            "UPDATE quotes SET span_end = 4, span_start = NULL",
            &[],
        )
        .unwrap();
        assert!(matches!(turns(&mut c, conv), Err(StoreError::Corrupt(_))));
        execute(
            &mut c,
            "UPDATE quotes SET span_start = NULL, span_end = NULL",
            &[],
        )
        .unwrap();
        assert!(matches!(turns(&mut c, conv), Err(StoreError::Corrupt(_))));
        execute(
            &mut c,
            "UPDATE quotes SET span_start = 0, span_end = 4",
            &[],
        )
        .unwrap();
        execute(&mut c, "UPDATE turns SET status = 'weird'", &[]).unwrap();
        assert!(matches!(turns(&mut c, conv), Err(StoreError::Corrupt(_))));
    }

    /// Status and mode round-trip through their stored text.
    #[test]
    fn status_and_mode_parse() {
        for s in [
            TurnStatus::Complete,
            TurnStatus::Stopped,
            TurnStatus::Failed,
        ] {
            assert_eq!(TurnStatus::parse(s.as_str()), Some(s));
        }
        for m in [Mode::WholePaper, Mode::Retrieval] {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert_eq!(TurnStatus::parse("x"), None);
        assert_eq!(Mode::parse("x"), None);
    }
}
