"""Reference SigLIP zero-shot scores with onnxruntime + transformers, to
check the Rust pipeline. Usage: siglip_reference.py <models_dir> <image> <prompt>..."""
import sys
import numpy as np
import onnxruntime as ort
from PIL import Image
from transformers import AutoTokenizer

models, image, prompts = sys.argv[1], sys.argv[2], sys.argv[3:]
tok = AutoTokenizer.from_pretrained("Xenova/siglip-base-patch16-224")
enc = tok(prompts, padding="max_length", max_length=64, truncation=True, return_tensors="np")
print("ids[0][:12]", enc["input_ids"][0][:12].tolist())
text = ort.InferenceSession(f"{models}/siglip-text-int8.onnx")
t = text.run(["pooler_output"], {"input_ids": enc["input_ids"].astype(np.int64)})[0]
img = Image.open(image).convert("RGB").resize((224, 224), Image.BICUBIC)
x = (np.asarray(img, dtype=np.float32) / 255.0 - 0.5) / 0.5
x = x.transpose(2, 0, 1)[None]
vision = ort.InferenceSession(f"{models}/siglip-vision-int8.onnx")
v = vision.run(["pooler_output"], {"pixel_values": x})[0]
t /= np.linalg.norm(t, axis=1, keepdims=True)
v /= np.linalg.norm(v, axis=1, keepdims=True)
logits = (t @ v.T)[:, 0] * np.exp(4.7649970054626465) - 12.9324369430542
for p, l in zip(prompts, logits):
    print(f"{1 / (1 + np.exp(-l)):.4f}  cos={(l + 12.93) / np.exp(4.765):.4f}  {p}")
