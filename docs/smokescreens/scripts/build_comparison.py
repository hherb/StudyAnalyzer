"""Usage: build_comparison.py PRIVATE_DIR (holding guidance_layout.json and bmlib.json; the page is
written there too, because it quotes the guidance and must not be committed).

Build the side-by-side page: RoB 2 guidance passage vs bmlib's wording, shared 5-word runs marked."""

import html
import json
import re
import sys
from pathlib import Path

N = 5  # length of a shared word run that counts as overlap
here = sys.argv[1]
guide = json.load(open(f"{here}/guidance_layout.json"))
ours = json.load(open(f"{here}/bmlib.json"))
domains = ours.pop("_domains")
version = ours.pop("_prompt_version", "?")
whole_guidance = " ".join(v["title"] + " " + " ".join(v["paras"]) for v in guide.values())

WORD = re.compile(r"[A-Za-z0-9]+(?:['’][A-Za-z]+)?")


def words(text):
    return [(m.start(), m.end(), m.group().lower().replace("’", "'")) for m in WORD.finditer(text)]


def grams(ws):
    return {tuple(w[2] for w in ws[i : i + N]) for i in range(len(ws) - N + 1)}


def marked(text, other_grams):
    """Escape text, wrapping every word inside a run shared with the other side in <mark>."""
    ws = words(text)
    hit = [False] * len(ws)
    runs = 0
    for i in range(len(ws) - N + 1):
        if tuple(w[2] for w in ws[i : i + N]) in other_grams:
            if not hit[i]:
                runs += 1
            for k in range(i, i + N):
                hit[k] = True
    out, pos, k = [], 0, 0
    while k < len(ws):
        if not hit[k]:
            k += 1
            continue
        j = k
        while j + 1 < len(ws) and hit[j + 1]:
            j += 1
        out.append(html.escape(text[pos : ws[k][0]]))
        out.append("<mark>" + html.escape(text[ws[k][0] : ws[j][1]]) + "</mark>")
        pos = ws[j][1]
        k = j + 1
    out.append(html.escape(text[pos:]))
    return "".join(out), runs


def ours_html(text):
    body, runs = marked(text, grams(words(whole_guidance)))
    lines = body.split("\n")
    rendered = []
    for i, line in enumerate(lines):
        m = re.match(r"^(Y|PY|PN|N|NI|NA|Look in|Applies only when):", line)
        if i == 0:
            rendered.append(f'<p class="q">{line}</p>')
        elif m:
            rendered.append(f'<p><b class="lab">{m.group(1)}</b>{line[len(m.group(1)) + 1:]}</p>')
        else:
            rendered.append(f"<p>{line}</p>")
    return "".join(rendered), runs


rows, total_ours, clean = [], 0, 0
current = None
for qid, text in ours.items():
    d = qid.split(".")[0]
    if d != current:
        current = d
        rows.append(f'<h2 id="d{d}">{html.escape(domains[d])}</h2>')
    ours_body, ours_runs = ours_html(text)
    g, theirs = guide[qid], grams(words(text))
    parts = [f'<p class="q">{marked(g["title"], theirs)[0]}</p>']
    for para in g["paras"]:
        body = marked(para, theirs)[0]
        body = re.sub(r"^(Answer ‘[^’]+’(?: or ‘[^’]+’)?(?: if:?)?|Also answer ‘[^’]+’)", r"<b>\1</b>", body)
        cls = ' class="item"' if re.match(r"^(\(\d+\)|\d\.\s|or\b)", para) else ""
        parts.append(f"<p{cls}>{body}</p>")
    parts.append(f'<p class="opts">Response options: <span>{html.escape(g["options"])}</span></p>')
    guide_body = "".join(parts)
    total_ours += ours_runs
    clean += ours_runs == 0
    chip = (
        '<span class="chip ok">no shared 5-word run</span>'
        if ours_runs == 0
        else f'<span class="chip warn">{ours_runs} shared run{"s" if ours_runs > 1 else ""}</span>'
    )
    rows.append(
        f"""<section class="row" id="q{qid}">
  <header class="rowhead"><span class="qid">{qid}</span>{chip}</header>
  <div class="cols">
    <div class="col guide"><div class="colhead">RoB 2 guidance, 2019 (verbatim)</div><div class="gtext">{guide_body}</div></div>
    <div class="col ours"><div class="colhead">bmlib, prompt version {version}</div>{ours_body}</div>
  </div>
</section>"""
    )

nav = " ".join(f'<a href="#q{q}">{q}</a>' for q in ours)
page = open(Path(__file__).with_name("comparison_template.html")).read()
page = (
    page.replace("{{ROWS}}", "\n".join(rows))
    .replace("{{NAV}}", nav)
    .replace("{{CLEAN}}", str(clean))
    .replace("{{RUNS}}", str(total_ours))
    .replace("{{VERSION}}", str(version))
)
open(f"{here}/rob2-wording.html", "w").write(page)
print("questions", len(ours), "clean", clean, "runs", total_ours)
