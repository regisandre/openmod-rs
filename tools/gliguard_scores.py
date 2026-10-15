"""Score a text corpus with the GLiGuard ONNX graph (Python reference) and
write one JSON line per text: id, lang, expected, P(unsafe) and the
toxicity label probabilities. Used to compare and combine text models
offline. Usage: gliguard_scores.py <model.onnx> <tokenizer.json> <corpus.jsonl> <out.jsonl>
"""
import json
import os
import sys

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gliguard_common import TOXICITY, build  # noqa: E402

model, tokenizer, corpus, out = sys.argv[1:5]
tok = Tokenizer.from_file(tokenizer)


class Tok:
    def encode(self, s, add_special_tokens=False):
        return tok.encode(s, add_special_tokens=add_special_tokens).ids


o = ort.SessionOptions()
o.intra_op_num_threads = 8
sess = ort.InferenceSession(model, o, providers=["CPUExecutionProvider"])
with open(out, "w", encoding="utf-8") as f:
    for line in open(corpus, encoding="utf-8"):
        if not line.strip():
            continue
        item = json.loads(line)
        ids, pos = build(Tok(), item["text"])
        logits = sess.run(None, {"input_ids": np.array([ids], dtype=np.int64),
                                 "positions": np.array(pos, dtype=np.int64)})[0]
        e = np.exp(logits[:2] - logits[:2].max())
        unsafe = float(e[1] / e.sum())
        tox = {label: float(1 / (1 + np.exp(-l))) for label, l in zip(TOXICITY, logits[2:])}
        f.write(json.dumps({"id": item["id"], "lang": item["lang"], "expected": item["expected"],
                            "categories": item.get("categories", []), "unsafe": unsafe, "tox": tox}) + "\n")
