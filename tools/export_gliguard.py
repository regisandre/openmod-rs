"""Export GLiGuard (GLiNER2 guardrails) classification to ONNX for openmod.

GLiNER2 classifies by encoding a schema and the text together:

    ( [P] task ( [L] label [L] label ... ) ) [SEP_STRUCT] ( [P] task2 ... ) [SEP_TEXT] words...

and scoring the encoder state of every `[L]` marker with a small MLP. The
exported graph takes the token ids and the `[L]` positions and returns one
logit per label:

    inputs:  input_ids [1, n], positions [k]
    output:  logits [k]

This script also re-implements the input builder that openmod uses in Rust,
checks it against the gliner2 library on sample texts, then quantizes the
embedding table to 4 bits and checks again. The matmuls stay fp32: 4-bit
matmuls moved some probabilities by more than 0.4 on this encoder, and the
8-bit kernels were slower than fp32 (see tools/quantize_gliguard.py).

Usage: export_gliguard.py <out_dir> [repo]
"""
import os
import sys

import numpy as np
import torch
from gliner2 import GLiNER2

out_dir = sys.argv[1]
repo = sys.argv[2] if len(sys.argv) > 2 else "fastino/GLiNER2-Guardrails-PII-Multi"
os.makedirs(out_dir, exist_ok=True)
fp32_path = os.path.join(out_dir, "gliguard-fp32.onnx")
q4_path = os.path.join(out_dir, "gliguard-emb4.onnx")

model = GLiNER2.from_pretrained(repo)
model.eval()
tok = model.processor.tokenizer
assert tok.is_fast, "a fast tokenizer is needed to write tokenizer.json"
tok.backend_tokenizer.save(os.path.join(out_dir, "gliguard-tokenizer.json"))

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gliguard_common import SAFETY, TASKS, TOXICITY, build as build_with  # noqa: E402


def build(text):
    return build_with(tok, text)


def reference(text):
    """Logits computed by the gliner2 library itself."""
    got = []
    hook = model.classifier.register_forward_hook(lambda mod, a, o: got.append(o.detach().flatten()))
    model.classify_text(text, {"prompt_safety": SAFETY,
                               "prompt_toxicity": {"labels": TOXICITY, "multi_label": True}})
    hook.remove()
    return torch.cat(got).numpy()


class Classify(torch.nn.Module):
    def __init__(self, encoder, classifier):
        super().__init__()
        self.encoder = encoder
        self.classifier = classifier

    def forward(self, input_ids, positions):
        hidden = self.encoder(input_ids=input_ids, attention_mask=torch.ones_like(input_ids)).last_hidden_state
        return self.classifier(hidden[0].index_select(0, positions)).squeeze(-1)


wrapper = Classify(model.encoder, model.classifier).eval()
samples = [
    "Killer lineup tonight, this set was sick!",
    "Je vais te retrouver et te casser la gueule.",
    "Alle Ausländer raus, dreckiges Pack",
    "Хто хоче купити таблетки? Пиши в особисті",
    "Ik wil mezelf iets aandoen na dit feest",
    "DM me on t.me/selltix for cheap tickets, 100% legit",
]

with torch.no_grad():
    for s in samples:
        ids, pos = build(s)
        mine = wrapper(torch.tensor([ids]), torch.tensor(pos)).numpy()
        diff = float(np.abs(mine - reference(s)).max())
        assert diff < 1e-3, f"input builder differs from gliner2 on {s!r}: {diff}"
    print("input builder matches gliner2 on", len(samples), "samples")

    ids, pos = build(samples[1])
    torch.onnx.export(
        wrapper,
        (torch.tensor([ids]), torch.tensor(pos)),
        fp32_path,
        input_names=["input_ids", "positions"],
        output_names=["logits"],
        dynamic_axes={"input_ids": {1: "seq"}, "positions": {0: "k"}, "logits": {0: "k"}},
        opset_version=17,
        dynamo=False,
        external_data=True,
    )
print("exported", fp32_path)

import onnx
import onnxruntime as ort
from onnxruntime.quantization.matmul_nbits_quantizer import DefaultWeightOnlyQuantConfig, MatMulNBitsQuantizer


def run(path, s):
    ids, pos = build(s)
    sess = run.cache.setdefault(path, ort.InferenceSession(path, providers=["CPUExecutionProvider"]))
    return sess.run(None, {"input_ids": np.array([ids], dtype=np.int64), "positions": np.array(pos, dtype=np.int64)})[0]


run.cache = {}
for s in samples:
    diff = float(np.abs(run(fp32_path, s) - reference(s)).max())
    assert diff < 1e-3, f"fp32 ONNX differs on {s!r}: {diff}"
print("fp32 ONNX matches gliner2")

cfg = DefaultWeightOnlyQuantConfig(block_size=32, is_symmetric=True, op_types_to_quantize=("Gather",),
                                   quant_axes=(("Gather", 0),))
q = MatMulNBitsQuantizer(onnx.load(fp32_path), algo_config=cfg)
q.process()
q.model.save_model_to_file(q4_path, use_external_data_format=False)
print("quantized", q4_path, os.path.getsize(q4_path) // 2**20, "MB")


def probs(logits):
    s = np.exp(logits[:2] - logits[:2].max())
    return np.concatenate([s / s.sum(), 1 / (1 + np.exp(-logits[2:]))])


worst = max(float(np.abs(probs(run(q4_path, s)) - probs(run(fp32_path, s))).max()) for s in samples)
print("emb4 vs fp32, worst probability difference on samples:", round(worst, 4))
