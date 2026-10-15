"""Compare quantized variants of the GLiGuard ONNX graph with fp32 on real
texts: probability error, decision agreement and CPU time.

Usage: quantize_gliguard.py <dir with gliguard-fp32.onnx + tokenizer> <corpus.jsonl> [variant ...]
Variants: q8, emb4_mm8, dyn8, emb4
"""
import json
import os
import sys
import time

import numpy as np
import onnx
import onnxruntime as ort
from tokenizers import Tokenizer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gliguard_common import build  # noqa: E402

d, corpus = sys.argv[1], sys.argv[2]
variants = sys.argv[3:] or ["q8", "emb4_mm8", "dyn8", "emb4"]
fp32 = os.path.join(d, "gliguard-fp32.onnx")
tok = Tokenizer.from_file(os.path.join(d, "gliguard-tokenizer.json"))


class Tok:
    """The subset of the transformers tokenizer API used by build()."""

    def encode(self, s, add_special_tokens=False):
        return tok.encode(s, add_special_tokens=add_special_tokens).ids


texts = [json.loads(l)["text"] for l in open(corpus, encoding="utf-8") if l.strip()]
texts = texts[:: max(1, len(texts) // 80)][:80]


def quantize(variant, out):
    from onnxruntime.quantization.matmul_nbits_quantizer import DefaultWeightOnlyQuantConfig, MatMulNBitsQuantizer

    def nbits(model, bits, ops):
        cfg = DefaultWeightOnlyQuantConfig(block_size=32, is_symmetric=True, bits=bits, op_types_to_quantize=ops,
                                           quant_axes=(("MatMul", 0), ("Gather", 0)))
        q = MatMulNBitsQuantizer(model, algo_config=cfg)
        q.process()
        return q.model.model

    if variant == "dyn8":
        from onnxruntime.quantization import QuantType, quantize_dynamic
        quantize_dynamic(fp32, out, weight_type=QuantType.QInt8, per_channel=True, op_types_to_quantize=["MatMul", "Gather"])
        return
    m = onnx.load(fp32)
    if variant == "q8":
        m = nbits(m, 8, ("MatMul", "Gather"))
    elif variant == "emb4_mm8":
        m = nbits(nbits(m, 8, ("MatMul",)), 4, ("Gather",))
    elif variant == "emb4":
        m = nbits(m, 4, ("Gather",))
    onnx.save(m, out)


def session(path, threads):
    o = ort.SessionOptions()
    o.intra_op_num_threads = threads
    o.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    return ort.InferenceSession(path, o, providers=["CPUExecutionProvider"])


def probs(logits):
    s = np.exp(logits[:2] - logits[:2].max())
    return np.concatenate([s / s.sum(), 1 / (1 + np.exp(-logits[2:]))])


def run_all(path):
    sess = session(path, 2)
    out, t = [], 0.0
    for text in texts:
        ids, pos = build(Tok(), text)
        feed = {"input_ids": np.array([ids], dtype=np.int64), "positions": np.array(pos, dtype=np.int64)}
        start = time.perf_counter()
        logits = sess.run(None, feed)[0]
        t += time.perf_counter() - start
        out.append(probs(logits))
    return np.array(out), t / len(texts) * 1000


ref, ms = run_all(fp32)
print(f"fp32: {os.path.getsize(fp32) // 2**20} MB, {ms:.0f} ms/text (2 threads), {len(texts)} texts")
for v in variants:
    path = os.path.join(d, f"gliguard-{v}.onnx")
    if not os.path.exists(path):
        quantize(v, path)
    got, ms = run_all(path)
    err = np.abs(got - ref)
    agree_unsafe = float(((got[:, 1] >= 0.5) == (ref[:, 1] >= 0.5)).mean())
    agree_tox = float(((got[:, 2:] >= 0.4) == (ref[:, 2:] >= 0.4)).mean())
    print(f"{v}: {os.path.getsize(path) // 2**20} MB, {ms:.0f} ms/text, max err {err.max():.3f}, "
          f"mean err {err.mean():.4f}, unsafe agree {agree_unsafe:.0%}, toxicity agree {agree_tox:.1%}")
