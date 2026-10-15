"""GLiGuard input builder shared by the export and quantization scripts.

Mirrors the Rust implementation in src/models/gliguard.rs.
"""
import re
import sys
import unicodedata

SAFETY = ["safe", "unsafe"]
TOXICITY = [
    "violence_and_weapons", "non_violent_crime", "sexual_content", "hate_and_discrimination",
    "self_harm_and_suicide", "pii_exposure", "misinformation", "copyright_violation", "child_safety",
    "political_manipulation", "unethical_conduct", "regulated_advice", "privacy_violation", "other", "benign",
]
TASKS = [("prompt_safety", SAFETY), ("prompt_toxicity", TOXICITY)]

# Same pattern as gliner2.processing.word_splitter.WhitespaceTokenSplitter.
MARKS = "".join(chr(c) for c in range(sys.maxunicode + 1) if unicodedata.category(chr(c)).startswith("M"))
WORD = rf"[\w{MARKS}\u200c\u200d]"
SPLIT = re.compile(
    rf"(?:https?://[^\s]+|www\.[^\s]+)|[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{{2,}}|@[a-z0-9_]+|{WORD}+(?:[-_]{WORD}+)*|\S",
    re.IGNORECASE,
)


def build(tok, text):
    """Token ids and [L] positions, exactly as openmod builds them in Rust."""
    if not text:
        text = "."
    elif not text.endswith((".", "!", "?")):
        text += "."
    pieces = []
    for i, (task, labels) in enumerate(TASKS):
        if i:
            pieces.append("[SEP_STRUCT]")
        pieces += ["(", "[P]", task, "("]
        for label in labels:
            pieces += ["[L]", label]
        pieces += [")", ")"]
    pieces.append("[SEP_TEXT]")
    pieces += [w.lower() for w in SPLIT.findall(text)]
    ids, positions = [], []
    for p in pieces:
        if p == "[L]":
            positions.append(len(ids))
        ids += tok.encode(p, add_special_tokens=False)
    return ids, positions
