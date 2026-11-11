"""Every arm F flag to check by hand: items "not reported" at both looks, and switches giving Y.

Run from docs/smokescreens with the bmlib venv's Python: <venv>/bin/python scripts/review_f.py
For each flag it prints the passage naming the item and the paper's other passages that mention
the item's distinctive words with a number in them, so a reader can see whether the result is
there after all.
"""

import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run import paper  # noqa: E402

DATA = Path(__file__).resolve().parent.parent / "data/2026-11-10-rob2-wording"
STOP = {"with", "from", "that", "this", "were", "after", "score", "scale", "analysis", "outcome",
        "change", "baseline", "between", "groups", "group", "patients", "measured", "months", "weeks"}


def mentions(text: str, label: str, limit: int = 3) -> list[str]:
    """Sentences of *text* holding a distinctive word of *label* and a digit."""
    words = [w for w in re.findall(r"[A-Za-z][A-Za-z\-]{3,}", label) if w.lower() not in STOP]
    found = []
    for sentence in re.split(r"(?<=[.;])\s+|\n", text):
        if any(w.lower() in sentence.lower() for w in words) and re.search(r"\d", sentence):
            found.append(sentence.strip()[:220])
        if len(found) >= limit:
            break
    return found


def main() -> None:
    texts: dict[str, str] = {}
    for folder in sorted(DATA.glob("results*-f")):
        for path in sorted(folder.glob("*_F?.json")):
            r = json.loads(path.read_text())
            if not r.get("lists"):
                continue
            text = texts.setdefault(r["rct"], paper(r["rct"])[1])
            head = f"{folder.name} {r['rct']} run {r['rep']} ({r['outcome'][:50]})"
            if r["d5"]["answers"]["5.2"]["answer"] == "Y":
                quotes = [c["text"][:200] for c in r["facts"]["switched"]["citations"]]
                print(f"\n{head}\n  SWITCHED: {quotes}")
            for key, items in r["lists"].items():
                for item in items:
                    if item["not_reported"]:
                        print(f"\n{head}\n  NOT REPORTED ({key}): {item['label']}")
                        print(f"    named: {[q[:200] for q in item['named']]}")
                        for m in mentions(text, item["label"]):
                            print(f"    mention: {m}")


if __name__ == "__main__":
    main()
