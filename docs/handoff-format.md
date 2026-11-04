# The StudyAnalyzer hand-off format

For people writing a sender. A hand-off is a `.studyanalyzer` file (JSON,
UTF-8) or a `studyanalyzer://open?…` link.

The file is one JSON object. A leading UTF-8 byte order mark (as Windows
tools write) is skipped; anything that does not then start, after
whitespace, with `{` (a top-level array, other text) is "not a hand-off".

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

- `format` must be the string `studyanalyzer-handoff`; anything else (missing,
  another type, another value) is "not a hand-off".
- `version` is a positive integer (any other value is refused as malformed);
  this app reads 1.
- `sender`: required, not blank, at most 200 characters, no control characters
  and no bidirectional embedding, override or isolate (U+202A–U+202E,
  U+2066–U+2069), which could make a name display reversed. The same
  character rules hold for the optional `full_text.file_name` and
  `full_text.source`.
- `title`: optional, at most 2000 characters; it may carry control and
  bidirectional characters (a right-to-left title needs them), and is shown
  isolated so they cannot reorder the text after it.
- An optional field (`title`, an identifier, `full_text.file_name`,
  `full_text.source`) that is `null`, empty or only whitespace counts as absent.
- `identifiers.pmid`: a PubMed ID, or a preprint accession (`PPR` + digits).
  `doi` and `pmcid` are checked as when typed. Each is at most 256
  characters (`MAX_IDENTIFIER_CHARS`) and is checked on its own; an invalid
  or longer one is dropped and the user is told. A hand-off needs a valid
  identifier or `full_text`, else it is refused.
- `full_text.data`: one base64 string, standard alphabet, padded, with no
  whitespace or line breaks; the file's own bytes for every format (text
  formats too; the app decodes them as if opened by hand). MIME-wrapped
  output, such as Python's `base64.encodebytes` or Java's
  `Base64.getMimeEncoder()`, is refused; Python's `base64.b64encode`, Swift's
  `Data.base64EncodedString()` (no options) and Java's `Base64.getEncoder()`
  are right. The kind comes from `full_text.format`, nothing else.
- `full_text.file_name`: a name, never a path: no `/`, `\`, control or
  bidirectional character, at most 200 characters. `full_text.source`: at
  most 200 characters, no control or bidirectional characters; shown, never
  followed.
- `identifiers` and `full_text` must be JSON objects. A wrong type is refused
  with a fixed message naming the field; the message never repeats the file's text.
- Nothing is truncated: a field over its cap refuses the whole file, naming
  the field. A duplicate known key (at the top level, in `identifiers` or in
  `full_text`) is refused; an unknown field is ignored, even repeated.
- The whole file is at most `MAX_HANDOFF_BYTES`, 140,858,712 bytes (about
  134.3 MiB: the largest PDF, 100 MiB, in base64, plus 1 MiB), checked before
  parsing.

## Versioning

Adding an optional field does not change `version`, and unknown fields are
ignored, so a newer sender still works with an older app. A breaking change
bumps `version`; an older app then refuses the file, saying so.

## The link

`studyanalyzer://open?pmid=…&doi=…&pmcid=…`, at most 2048 characters. Values
are percent-encoded (a DOI's `/` is `%2F`). A literal `+` decodes to a space,
so encode a `+` inside a value as `%2B`. A repeated key or an empty value is
refused, as is a fragment (`#…`; encode a `#` inside a value as `%23`), a user
name, password, port, or any path (an empty path or a lone `/` is fine); an unknown key is ignored. A link carries identifiers only: no sender,
no title, no text.

## Fixtures

`crates/studyanalyzer-core/tests/fixtures/handoff/`: `cases.json` lists each
`*.studyanalyzer` file with its expected outcome. One that parses carries
no notices, and its `bytes` names the fixture paper (one folder up) its
`full_text.data` encodes (null when it carries no text). A sender's tests
can read them to check a reader, or write the same shapes and compare. The reference
parser is `crates/studyanalyzer-core/src/handoff.rs`.
