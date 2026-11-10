"""Replay RCT081's D2 call (wording A) once with a high token limit; report reply length and usage."""
import json, sys
sys.argv = ["x"]
import run, bmlib.quality.rob2_agent as ra
from bmlib.llm.client import LLMClient
from bmlib.llm import LLMMessage
from bmlib.quality.outcome_agent import ProposedOutcome
from bmlib.quality.rob2_questions import Domain
run.local.arm = "A"
s = next(x for x in json.load(open(run.DATA / "sample.json")) if x["rct"] == "RCT081")
title, text = run.paper("RCT081")
prompt = ra.domain_prompt(Domain.D2, title, text, ProposedOutcome(name=s["outcome"]))
r = LLMClient().chat([LLMMessage(role="system", content=ra.ROB2_SYSTEM_PROMPT), LLMMessage(role="user", content=prompt)],
                     model=run.MODEL, temperature=0.1, max_tokens=32768, json_mode=True)
c = r.content or ""
print("stop:", getattr(r, "stop_reason", None), "| content chars:", len(c), "| usage:", getattr(r, "usage", None) or {k: getattr(r, k, None) for k in ("input_tokens", "output_tokens")})
print("reasoning chars:", len(getattr(r, "reasoning", "") or getattr(r, "thinking", "") or ""))
try:
    d = json.loads(c); a = d.get("answers", {})
    print({q: (v.get("answer"), len(v.get("quotes", [])), sum(map(len, v.get("quotes", [])))) for q, v in a.items()})
except Exception as e:
    print("not JSON:", e, repr(c[-400:]))
