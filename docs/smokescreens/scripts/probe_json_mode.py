"""Send RCT045's D1 request (wording C) to an OpenAI-compatible server as is, with and without JSON
mode, and save both raw replies. It is how Qwen3.8's "[1.1]" replies under oMLX were traced to
JSON mode. Usage: probe_json_mode.py [model] [base_url]
"""

import json
import sys
import time
import urllib.request

MODEL = sys.argv[1] if len(sys.argv) > 1 else "Qwen3.8-27B-8bit"
BASE_URL = sys.argv[2] if len(sys.argv) > 2 else "http://127.0.0.1:8000/v1"
sys.argv = ["x"]  # run.py reads its arms from argv at import
import bmlib.quality.rob2_agent as ra  # noqa: E402
import run  # noqa: E402
from bmlib.quality.rob2_questions import Domain  # noqa: E402

OUT = run.DATA / "json-mode-probe"
run.local.arm = "C"
title, text = run.paper("RCT045")
messages = [{"role": "system", "content": ra.ROB2_SYSTEM_PROMPT},
            {"role": "user", "content": ra.domain_prompt(Domain.D1, title, text, None)}]
OUT.mkdir(exist_ok=True)
for json_mode in (True, False):
    body = {"model": MODEL, "temperature": 0.1, "max_tokens": 16384,
            "messages": messages}
    if json_mode:
        body["response_format"] = {"type": "json_object"}
    started = time.monotonic()
    req = urllib.request.Request(f"{BASE_URL}/chat/completions", json.dumps(body).encode(),
                                 {"Content-Type": "application/json", "Authorization": "Bearer local"})
    reply = json.load(urllib.request.urlopen(req, timeout=3600))
    name = f"{MODEL}-{'json-mode' if json_mode else 'no-json-mode'}.json"
    (OUT / name).write_text(json.dumps(reply, ensure_ascii=False, indent=1))
    content = reply["choices"][0]["message"].get("content") or ""
    print(name, f"{time.monotonic() - started:.0f}s", reply.get("usage", {}).get("completion_tokens_details"),
          f"{len(content)} chars:", content[:120].replace("\n", " "), flush=True)
