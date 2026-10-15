"""Compile tools/corpus/text/*.txt into testdata/text/corpus.jsonl.

Usage: python -I build_text_corpus.py <src_dir> <out_file>
"""
import json, os, sys, collections

SRC, OUT = sys.argv[1], sys.argv[2]
LANGS = ["de", "en", "es", "fr", "it", "nl", "uk"]
E = {"s": "safe", "r": "review", "x": "reject"}
CTX = {"b": "bio", "e": "event_description", "c": "comment", "h": "chat"}
CATS = {"hate", "harassment", "sexual", "violence", "self_harm", "spam", "scam", "pii", "drugs", "illegal"}
rows, counter = [], collections.Counter()

def add(lang, e, cats, ctx, text, tag):
    cats = [] if cats == "-" else cats.split(",")
    assert set(cats) <= CATS, (cats, text)
    assert not (e == "s" and cats), text
    assert not (e != "s" and not cats), text
    assert chr(0x2014) not in text and chr(0x2013) not in text, text
    counter[lang] += 1
    rows.append({"id": f"{tag}-{lang}-{counter[lang]:03d}", "lang": lang, "text": text.strip(),
                 "expected": E[e], "categories": cats, "context": CTX[ctx]})

for lang in LANGS:
    for line in open(os.path.join(SRC, lang + ".txt"), encoding="utf-8"):
        if not line.strip() or line.startswith("#"): continue
        e, cats, ctx, text = line.rstrip("\n").split("|", 3)
        add(lang, e, cats, ctx, text, "own")
for line in open(os.path.join(SRC, "mixed.txt"), encoding="utf-8"):
    if not line.strip() or line.startswith("#"): continue
    lang, e, cats, ctx, text = line.rstrip("\n").split("|", 4)
    add(lang, e, cats, ctx, text, "mix")

with open(OUT, "w", encoding="utf-8", newline="\n") as f:
    for r in rows:
        f.write(json.dumps(r, ensure_ascii=False) + "\n")
by = collections.Counter((r["lang"], r["expected"]) for r in rows)
for lang in LANGS:
    print(lang, {k: by[(lang, k)] for k in E.values()})
print("total", len(rows))
