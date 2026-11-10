"""Usage: export_bmlib_questions.py OUT_JSON

Write bmlib's 22 RoB 2 question texts and domain titles as JSON, for build_comparison.py.
Run with a Python that has bmlib from branch feat/evidence-weight installed.
"""

import json
import sys

from bmlib.quality import rob2_questions as q

IDS = [f"{d}.{i}" for d, n in [(1, 3), (2, 7), (3, 4), (4, 5), (5, 3)] for i in range(1, n + 1)]
out = {i: q.question_text(i) for i in IDS}
out["_domains"] = {str(k + 1): q.domain_title(d) for k, d in enumerate(q.Domain)}
out["_prompt_version"] = q.ROB2_PROMPT_VERSION
json.dump(out, open(sys.argv[1], "w"), ensure_ascii=False, indent=1)
