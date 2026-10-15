"""Score the whole text pipeline offline, the way the engine combines it:
rules (PII, spam, off-platform contact, profanity) + the first model + the
second model as a second opinion when the first model's `text_unsafe` is at
least `escalate`, with categories capped at `text_unsafe`.

Usage: system_text.py <rules.jsonl> <first.jsonl> <second.jsonl> [escalate=0.2] [rule set]
Rule sets: "all" (default) or "no_profanity". Prints caught / safe flagged
for a sweep of `text_unsafe` thresholds, overall and per language.
"""
import json
import sys
from collections import defaultdict


def load(path):
    return {r["id"]: r for r in map(json.loads, open(path, encoding="utf-8")) if r.get("kind") == "text"}


rules, first, second = (load(p) for p in sys.argv[1:4])
escalate = float(sys.argv[4]) if len(sys.argv) > 4 else 0.2
rule_set = sys.argv[5] if len(sys.argv) > 5 else "all"
CERTAIN = ["pii_email", "pii_phone", "pii_iban", "pii_card", "messaging_handoff", "spam", "profanity"]
if rule_set == "no_profanity":
    CERTAIN.remove("profanity")
ids = [i for i in first if i in second and i in rules]


def rule_hit(i):
    return any(rules[i]["scores"].get(k, 0) >= 1.0 for k in CERTAIN)


def verdict(i):
    a = first[i]["scores"].get("text_unsafe", 0.0)
    if a < escalate:
        return a
    return (a + second[i]["scores"].get("text_unsafe", 0.0)) / 2


bad = [i for i in ids if first[i]["expected"] != "safe"]
safe = [i for i in ids if first[i]["expected"] == "safe"]
runs = sum(first[i]["scores"].get("text_unsafe", 0.0) >= escalate for i in ids) / len(ids)
print(f"{len(ids)} texts ({len(bad)} violating, {len(safe)} safe); second model runs on {runs:.0%}; rules: {rule_set}")
print(f"{'thr':>5}{'caught':>8}{'reject':>8}{'review':>8}{'safe flagged':>14}")
for t in (0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9):
    flag = lambda i: rule_hit(i) or verdict(i) >= t  # noqa: E731
    rej = [i for i in bad if first[i]["expected"] == "reject"]
    rev = [i for i in bad if first[i]["expected"] == "review"]
    pct = lambda xs: sum(map(flag, xs)) / max(len(xs), 1)  # noqa: E731
    print(f"{t:>5.1f}{pct(bad):>8.0%}{pct(rej):>8.0%}{pct(rev):>8.0%}{pct(safe):>14.1%}")

t = float(sys.argv[6]) if len(sys.argv) > 6 else 0.6
print(f"\nper language at {t}:")
by = defaultdict(lambda: [0, 0, 0, 0])
for i in ids:
    b = by[first[i]["lang"]]
    hit = rule_hit(i) or verdict(i) >= t
    if first[i]["expected"] == "safe":
        b[2] += 1
        b[3] += hit
    else:
        b[0] += 1
        b[1] += hit
for lang, (n, c, s, h) in sorted(by.items()):
    print(f"  {lang}: caught {c}/{n} ({c / n:.0%}), safe flagged {h}/{s} ({h / s:.0%})")
