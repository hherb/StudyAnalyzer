# Test fixtures

- `article.xml`, `page.html`, `paper.md`, `paper.txt`, `paper.pdf`,
  `pdf_warnings_cut_short.pdf`: small synthetic papers written for this
  repository.
- `real_rct.xml`: a real paper, used by `tests/quote_threshold.rs` to measure
  the quote-acceptance threshold. JATS XML from Europe PMC.
  - Title: Erector spinae plane block added to intercostal nerve block versus
    intercostal nerve block alone for postoperative analgesia in
    video-assisted thoracoscopic surgery: A randomized controlled trial.
  - Journal: PLoS One, 2026. PMC ID: PMC13712513. PMID: 42945162.
  - Licence: Creative Commons Attribution 4.0 (CC BY 4.0),
    https://creativecommons.org/licenses/by/4.0/ . The text is unchanged; the
    authors retain copyright.
- `quotes.json`: hand-labelled quotes. Each case is
  `{ "paper": "article.xml" | "real_rct.xml", "quote", "accept", "why" }`; the
  top-level `papers` lists the papers used. `accept: true` only for the
  paper's own words, near-verbatim (case, whitespace, punctuation, dashes,
  accents); a changed number, direction, negation or merged text is `false`.
