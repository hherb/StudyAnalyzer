# The StudyAnalyzer hand-off format

For people writing a sender. A hand-off is a `.studyanalyzer` file (JSON,
UTF-8) or a `studyanalyzer://open?…` link.

## The file

```json
{
  "format": "studyanalyzer-handoff",
  "version": 1,
  "sender": "MedicalFactChecker",
  "title": "Optional title",
  "identifiers": { "pmid": "…", "doi": "…", "pmcid": "…" },
  "full_text": {
    "format": "pdf | jats | html | markdown | text",
    "data": "<base64 of the file's bytes>",
    "file_name": "optional, a name only",
    "source": "optional, e.g. Europe PMC"
  }
}
```

- `format` must be `studyanalyzer-handoff`, else the file is "not a hand-off".
- `version` is an integer; this app reads 1.
- `sender`: required, not blank, at most 200 characters, no control characters.
- `title`: optional, at most 2000 characters.
- `identifiers.pmid`: a PubMed ID, or a preprint accession (`PPR` + digits).
  `doi` and `pmcid` are checked as when typed. Each is checked on its own; an
  invalid one is dropped and the user is told. A hand-off needs a valid
  identifier or `full_text`, else it is refused.
- `full_text.data`: standard base64 alphabet, padded; the file's own bytes
  for every format (text formats too; the app decodes them as if opened by
  hand). The kind comes from `full_text.format`, nothing else.
- `full_text.file_name`: a name, never a path: no `/`, `\`, or control
  character, at most 200 characters. `full_text.source`: at most 200
  characters, no control characters; shown, never followed.
- Nothing is truncated: a field over its cap refuses the whole file, naming
  the field. A duplicate key is refused.
- The whole file is at most `MAX_HANDOFF_BYTES` (the largest PDF, 100 MB, in
  base64, plus 1 MiB), checked before parsing.

## Versioning

Adding an optional field does not change `version`, and unknown fields are
ignored, so a newer sender still works with an older app. A breaking change
bumps `version`; an older app then refuses the file, saying so.

## The link

`studyanalyzer://open?pmid=…&doi=…&pmcid=…`, at most 2048 characters. Values
are percent-encoded (a DOI's `/` is `%2F`). A repeated key or an empty value is
refused; an unknown key is ignored. A link carries identifiers only: no sender,
no title, no text.

## Fixtures

`crates/studyanalyzer-core/tests/fixtures/handoff/`: `cases.json` lists each
`*.studyanalyzer` file with its expected outcome. A sender's tests can read
them to check a reader, or write the same shapes and compare. The reference
parser is `crates/studyanalyzer-core/src/handoff.rs`.
