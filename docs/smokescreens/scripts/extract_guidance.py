"""Usage: extract_guidance.py ROB2_LAYOUT_TXT OUT_JSON
(ROB2_LAYOUT_TXT: `pdftotext -layout` of the RoB 2 full guidance PDF, 22 August 2019.
The table line numbers below are for that file.)

Rebuild each RoB 2 signalling question from the layout text: question, guidance paragraphs, options.

The PDF table has three columns (question | elaboration | response options); pdftotext -layout keeps
them at fixed character positions, so each line is split by column instead of read straight across.
"""

import collections
import json
import re
import sys

lines = open(sys.argv[1]).read().split("\n")
TABLES = [(795, 852), (1276, 1331), (1959, 2006), (2321, 2352), (2639, 2722)]  # 1-based header, end
OPTIONS = re.compile(r"\s{2,}((?:NA/)?Y/PY/PN/N(?:/NI)?)\s*$")
ROW = re.compile(r"^(\d\.\d)\.?\s")
SKIP = re.compile(r"^\s*(\d{1,3}|Box \d.*|Signalling questions.*|Response options)\s*$")

out = {}
for start, end in TABLES:
    body = lines[start : end - 1]  # after the header line, before "Risk-of-bias judgement"
    cols = collections.Counter(
        len(l) - len(l.lstrip()) for l in body if l.strip() and len(l) - len(l.lstrip()) > 10
    )
    elab = cols.most_common(1)[0][0]
    qid, title, paras, opts, cur = None, [], [], "", []

    def flush():
        if qid:
            if cur:
                paras.append(cur[:])
            out[qid] = {"title": title[:], "options": opts, "paras": [p for p in paras if p]}

    for raw in body:
        if SKIP.match(raw) or (raw.strip().startswith("Box ") and len(raw) - len(raw.lstrip()) < 4):
            continue
        line = raw.rstrip()
        m = OPTIONS.search(line)
        o = ""
        if m:
            o, line = m.group(1), line[: m.start()]
        if len(line) - len(line.lstrip()) >= 10:  # the question column starts at 0-2; this is guidance text
            left, mid = "", line
        elif line[:elab].strip() and len(line) > elab and line[elab - 1] != " ":
            cut = line.find("  ", 2)  # the left text ran past the column; split at its gap
            left, mid = line[:cut], line[cut:]
        else:
            left, mid = line[:elab], line[elab:]
        left, mid = left.strip(), mid.strip()
        r = ROW.match(left)
        if r:
            flush()
            qid, title, paras, cur, opts = r.group(1), [left[r.end() :].strip()], [], [], o
            if mid:
                cur.append(mid)
            continue
        if qid is None:
            continue
        if o and not opts:
            opts = o
        if left:
            title.append(left)
        if not mid:
            if cur and re.search(r"[.:;?)]$", cur[-1]):  # a blank guidance line ends a paragraph only after a sentence
                paras.append(cur)
                cur = []
            continue
        if re.match(r"^\(\d+\)|^\([ivx]+\)|^\d\.\s|^Answer ‘|^Answer '|^or$", mid) and cur:
            paras.append(cur)
            cur = []
        cur.append(mid)
    flush()


def join(parts):
    text = ""
    for p in parts:
        if text.endswith("-") and p[:1].islower():
            text += p
        else:
            text = (text + " " + p).strip()
    return re.sub(r"\s+", " ", text)


final = {
    q: {"title": join(v["title"]), "options": v["options"], "paras": [join(p) for p in v["paras"]]}
    for q, v in out.items()
}
stem_at = final["5.1"]["title"].find("Is the numerical result")
if stem_at > 0:  # in the PDF the stem of 5.2 and 5.3 sits in 5.1's question cell
    stem = final["5.1"]["title"][stem_at:].removesuffix("...").strip()
    final["5.1"]["title"] = final["5.1"]["title"][:stem_at].strip()
    for q in ("5.2", "5.3"):
        final[q]["title"] = stem + " " + final[q]["title"].removeprefix("...").strip()
json.dump(final, open(sys.argv[2], "w"), ensure_ascii=False, indent=1)
for q, v in final.items():
    print(q, "|", v["title"], "|", v["options"], "|", len(v["paras"]), "paras |", v["paras"][-1][-70:] if v["paras"] else "NONE")
