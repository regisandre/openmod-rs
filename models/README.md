# models

> Status: download target; the model files themselves are not committed.
> Last checked: 2026-10-16

`openmod fetch` (or the admin interface) fills this folder with the files of
the enabled tools. Every file comes from a pinned source and is kept only
when its size and SHA-256 match the values in `src/tools.rs`.

| File | Source |
|---|---|
| `marqo-nsfw-384.onnx` | Marqo/nsfw-image-detection-384, ONNX by KanariKanaru |
| `owen-safety-xs.onnx` | OwenElliott/image-safety-classifier-xs |
| `siglip-vision-int8.onnx`, `siglip-text-int8.onnx`, `siglip-tokenizer.json` | google/siglip-base-patch16-224, ONNX by Xenova |
| `gliguard-emb4.onnx`, `gliguard-tokenizer.json` | fastino/GLiNER2-Guardrails-PII-Multi, built by `tools/export_gliguard.py`, published with the `models-v1` release |
| `qwen3guard-q4.onnx` | Qwen/Qwen3Guard-Gen-0.6B, built by `tools/export_qwen3guard.py`, published with the `models-v1` release |
| `qwen3guard-tokenizer.json` | Qwen/Qwen3Guard-Gen-0.6B |

`sh scripts/build-text-models.sh` rebuilds the derived files from the
original weights. Licenses are listed in `NOTICE`.
