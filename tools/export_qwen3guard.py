"""Export Qwen3Guard-Gen to ONNX for openmod, then quantize it to 4 bits.

The prompt head (the fixed safety policy, about 200 tokens) is the same for
every check, so its key/value cache is computed once here and stored in the
graph as constants. At runtime the graph only processes the user text and
the fixed tail, and returns the logits at the requested positions only:

    inputs:  input_ids [1, n] (tokens after the head), positions [k]
    output:  logits [k, vocab]

One pass over `text + tail + " Unsafe\nCategories:"` gives the
Safe / Unsafe / Controversial distribution (at the position of `Safety:`)
and the first category token (at the last position), without generation.

Usage: export_qwen3guard.py <out_dir> [repo]
"""
import os
import sys

import numpy as np
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, DynamicCache

HEAD = open(os.path.join(os.path.dirname(__file__), "qwen3guard_head.txt"), encoding="utf-8").read()

out_dir = sys.argv[1]
repo = sys.argv[2] if len(sys.argv) > 2 else "Qwen/Qwen3Guard-Gen-0.6B"
os.makedirs(out_dir, exist_ok=True)
fp32_path = os.path.join(out_dir, "qwen3guard-fp32.onnx")
q4_path = os.path.join(out_dir, "qwen3guard-q4.onnx")

tok = AutoTokenizer.from_pretrained(repo)
model = AutoModelForCausalLM.from_pretrained(repo, torch_dtype=torch.float32, attn_implementation="eager").eval()
head_ids = torch.tensor([tok.encode(HEAD, add_special_tokens=False)])
P = head_ids.shape[1]
with torch.no_grad():
    cache = model(input_ids=head_ids, use_cache=True).past_key_values
layers = [(cache.layers[i].keys.clone(), cache.layers[i].values.clone()) for i in range(len(cache.layers))] \
    if hasattr(cache, "layers") else [(k.clone(), v.clone()) for k, v in cache.to_legacy_cache()]
print("head tokens:", P, "layers:", len(layers))


class AfterHead(torch.nn.Module):
    def __init__(self, m, layers):
        super().__init__()
        self.m = m
        for i, (k, v) in enumerate(layers):
            self.register_buffer(f"k{i}", k)
            self.register_buffer(f"v{i}", v)
        self.n = len(layers)

    def forward(self, input_ids, positions):
        cache = DynamicCache()
        for i in range(self.n):
            cache.update(getattr(self, f"k{i}"), getattr(self, f"v{i}"), i)
        n = input_ids.shape[1]
        position_ids = torch.arange(P, P + n).unsqueeze(0)
        mask = torch.ones((1, P + n), dtype=torch.long)
        out = self.m.model(input_ids=input_ids, attention_mask=mask, position_ids=position_ids,
                           past_key_values=cache, use_cache=True)
        picked = out.last_hidden_state[0].index_select(0, positions)
        return self.m.lm_head(picked)


wrapper = AfterHead(model, layers).eval()
ids = torch.randint(0, 50_000, (1, 48))
pos = torch.tensor([20, 47])
with torch.no_grad():
    torch.onnx.export(wrapper, (ids, pos), fp32_path, input_names=["input_ids", "positions"], output_names=["logits"],
                      dynamic_axes={"input_ids": {1: "seq"}, "positions": {0: "k"}, "logits": {0: "k"}},
                      opset_version=18, dynamo=False, external_data=True)
print("exported", fp32_path)

# Check against the full, uncached model on a real prompt of another length.
import onnxruntime as ort

text = " Je vais te retrouver et te casser la gueule.\n\n<END CONVERSATION>"
rest = torch.tensor([tok.encode(text, add_special_tokens=False)])
with torch.no_grad():
    full = model(input_ids=torch.cat([head_ids, rest], 1), use_cache=False).logits[0, -1].numpy()
got = ort.InferenceSession(fp32_path).run(None, {"input_ids": rest.numpy(), "positions": np.array([rest.shape[1] - 1])})[0][0]
print("fp32 max abs diff vs full model:", float(np.abs(full - got).max()))

import onnx
from onnxruntime.quantization.matmul_nbits_quantizer import DefaultWeightOnlyQuantConfig, MatMulNBitsQuantizer

cfg = DefaultWeightOnlyQuantConfig(block_size=32, is_symmetric=True, op_types_to_quantize=("MatMul", "Gather"),
                                   quant_axes=(("MatMul", 0), ("Gather", 0)))
q = MatMulNBitsQuantizer(onnx.load(fp32_path), algo_config=cfg)
q.process()
q.model.save_model_to_file(q4_path, use_external_data_format=False)
got4 = ort.InferenceSession(q4_path).run(None, {"input_ids": rest.numpy(), "positions": np.array([rest.shape[1] - 1])})[0][0]
print("quantized", q4_path, os.path.getsize(q4_path) // 2**20, "MB; top-5 overlap with fp32:",
      len(set(np.argsort(-full)[:5]) & set(np.argsort(-got4)[:5])), "/ 5")
