"""Make a labelled contact sheet for manual review: python -I contact_sheet.py <images_dir> <category> <out.jpg>"""
import os, sys
from PIL import Image, ImageDraw
root, cat, out = sys.argv[1:4]
fs = sorted(os.listdir(os.path.join(root, cat)))
fs = [f for f in fs if f.endswith(".jpg")]
T, cols = 160, 8
rows = (len(fs) + cols - 1) // cols
sheet = Image.new("RGB", (cols * T, rows * (T + 16)), "white"); d = ImageDraw.Draw(sheet)
for i, f in enumerate(fs):
    im = Image.open(os.path.join(root, cat, f)); im.thumbnail((T, T))
    x, y = (i % cols) * T, (i // cols) * (T + 16)
    sheet.paste(im, (x, y)); d.text((x + 2, y + T + 2), f.split("_")[-1][:3], fill="black")
sheet.save(out, quality=80)
