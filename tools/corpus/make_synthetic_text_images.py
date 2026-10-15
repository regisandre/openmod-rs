"""Render synthetic posters and chat screenshots with insults, phone numbers and scam links.

All content is generated here (no third-party material), so these images are CC0.
Usage: uv run --with pillow python -I make_synthetic_text_images.py <images_dir>
"""
import json, os, sys
from PIL import Image, ImageDraw, ImageFont

root = sys.argv[1]
out = os.path.join(root, "text_in_image"); os.makedirs(out, exist_ok=True)

def font(size, bold=False):
    for n in (["arialbd.ttf", "DejaVuSans-Bold.ttf"] if bold else ["arial.ttf", "DejaVuSans.ttf"]):
        try: return ImageFont.truetype(n, size)
        except OSError: pass
    return ImageFont.load_default()

def wrap(d, text, f, width):
    lines, cur = [], ""
    for w in text.split():
        t = (cur + " " + w).strip()
        if d.textlength(t, font=f) <= width: cur = t
        else: lines.append(cur); cur = w
    if cur: lines.append(cur)
    return lines

def poster(bg, fg, title, body, size=(900, 1200)):
    im = Image.new("RGB", size, bg); d = ImageDraw.Draw(im)
    y = 80
    for l in wrap(d, title, font(78, True), size[0] - 120):
        d.text((60, y), l, font=font(78, True), fill=fg); y += 95
    y += 40
    for l in wrap(d, body, font(46), size[0] - 120):
        d.text((60, y), l, font=font(46), fill=fg); y += 62
    return im

def chat(lines, size=(900, 700)):
    im = Image.new("RGB", size, (240, 240, 245)); d = ImageDraw.Draw(im)
    y = 30
    for who, text in lines:
        mine = who == "me"
        f = font(36)
        ls = wrap(d, text, f, 560)
        h = 28 + 46 * len(ls)
        x0 = size[0] - 640 if mine else 40
        d.rounded_rectangle((x0, y, x0 + 600, y + h), 24, fill=(120, 170, 255) if mine else (255, 255, 255))
        for i, l in enumerate(ls):
            d.text((x0 + 22, y + 14 + 46 * i), l, font=f, fill=(20, 20, 20))
        y += h + 22
    return im

ITEMS = [
 ("reject", "insult", poster((20, 20, 20), (255, 60, 60), "YOU ARE ALL LOSERS", "Whoever comes to this party is a worthless idiot. Go to hell and stay there.")),
 ("reject", "insult_de", poster((255, 235, 0), (0, 0, 0), "ALLE HIER SIND VOLLIDIOTEN", "Du bist eine hässliche Missgeburt und niemand will dich hier. Verpiss dich!")),
 ("reject", "phone", poster((255, 255, 255), (0, 0, 120), "CHEAP TICKETS", "Call now +44 7700 900456 or WhatsApp 0033 6 12 34 56 78. Cash only, no questions.")),
 ("reject", "scam_url", poster((0, 90, 60), (255, 255, 255), "FREE VIP TICKETS", "Claim yours now: http://free-vip-tix.example-scam.xyz - enter your card number to verify your age.")),
 ("reject", "drug_menu", poster((30, 0, 50), (255, 220, 0), "PARTY MENU", "MDMA 40 / gram, Coke 70 / gram, Pills 8 each. Telegram @plug_example, delivery to the club.")),
 ("reject", "iban", poster((245, 245, 245), (10, 10, 10), "DEPOSIT HERE", "Send 50 euro to IBAN BE68 5390 0754 7034 and message us your passport photo.")),
 ("reject", "chat_threat", chat([("them", "I know where you live"), ("them", "Come to the show and I will break your legs"), ("me", "please stop messaging me")])),
 ("reject", "chat_sexual_scam", chat([("them", "hey beautiful, send nudes and I put you on the VIP list"), ("them", "or pay 50 euro via http://vip-pay.example-scam.xyz"), ("me", "no thanks")])),
 ("reject", "chat_phone_pii", chat([("them", "his number is 06 12 34 56 78"), ("them", "his address is 12 Example Street, Brussels"), ("them", "everyone spam him")])),
 ("reject", "insult_fr", poster((120, 0, 0), (255, 255, 255), "VOUS ETES NULS", "Casse-toi sale connard, personne ne veut de toi ici. Va crever.")),
 ("safe", "control_event", poster((15, 15, 40), (255, 255, 255), "SATURDAY NIGHT", "Doors 22:00, three floors, eight DJs. Tickets at the door. Be kind to each other.")),
 ("safe", "control_chat", chat([("them", "see you at the show tonight?"), ("me", "yes! meet at the main stage at 9"), ("them", "perfect, bringing earplugs")])),
]
mp = os.path.join(root, "manifest.jsonl")
rows = [json.loads(l) for l in open(mp, encoding="utf-8")]
rows = [r for r in rows if not r["source_url"].startswith("generated:")]
for f in os.listdir(out):
    if f.startswith("synthetic_"): os.remove(os.path.join(out, f))
for exp, name, im in ITEMS:
    im.thumbnail((1024, 1024))
    fn = f"synthetic_{name}.jpg"
    im.save(os.path.join(out, fn), "JPEG", quality=85)
    rows.append({"path": f"text_in_image/{fn}", "category": "text_in_image", "expected": exp,
                 "source_url": "generated:tools/corpus/make_synthetic_text_images.py", "license": "CC0",
                 "author": "openmod-rs corpus (synthetic)", "title": name})
open(mp, "w", encoding="utf-8", newline="\n").write("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
print(len(ITEMS))
