"""Build the image corpus from Wikimedia Commons.

Usage: uv run --with requests --with pillow python -I fetch_commons_images.py <images_dir> [cat1,cat2]

Each category lists search queries; candidates are filtered on license, mime type,
size and a block-list of words that could indicate minors or explicit content.
Images are downloaded as <=1024 px thumbnails and re-encoded as JPEG q85.
"""
import io, json, os, re, sys, time, html
import requests
from PIL import Image

OUT = sys.argv[1]
ONLY = set(sys.argv[2].split(",")) if len(sys.argv) > 2 else None
API = "https://commons.wikimedia.org/w/api.php"
S = requests.Session()
# Wikimedia asks every API client for a way to reach its operator.
CONTACT = os.environ.get("OPENMOD_CORPUS_CONTACT") or sys.exit("set OPENMOD_CORPUS_CONTACT to an email or URL")
S.headers["User-Agent"] = f"openmod-rs-corpus/0.1 ({CONTACT})"

BLOCK = re.compile(r"\b(child|children|kid|kids|boy|boys|girl|girls|baby|babies|infant|toddler|teen|teens|teenager|"
                   r"minor|minors|putto|putti|cupid|cupids|amor|school|schoolgirl|schoolboy|youth|juvenile|young|"
                   r"pornograph\w*|porn|erotic|explicit|sex|sexual|fetish|bdsm|genital\w*|penis|vagina|"
                   r"enfant|kind|kinder|nino|nina|bambino|bambini)\b", re.I)
# For the minors group: anything that could show a child undressed is left out.
BLOCK_BARE = re.compile(r"\b(nude|nudes|nudity|naked|bare|bath|bathing|bathtub|shower|swim|swimming|swimsuit|"
                        r"bikini|beach|pool|underwear|diaper|diapers|topless|shirtless|pornograph\w*|porn|erotic|"
                        r"explicit|sex|sexual|fetish|genital\w*|abuse)\b", re.I)
BLOCK_MINORS = re.compile(r"\b(child|children|kid|kids|boy|boys|girl|girls|baby|babies|infant|toddler|teen|teens|"
                          r"teenager|minor|minors|youth|juvenile|young|schoolgirl|schoolboy)\b", re.I)

# category -> (expected, target, [queries])
CATS = {
 "safe_event": ("safe", 80, [
   "DJ performing nightclub", "DJ turntables booth", "music festival main stage crowd", "concert crowd hands up",
   "techno club dance floor", "electronic music festival", "rave party lights", "nightclub interior",
   "festival food stall street food", "people dancing party", "concert poster", "festival poster",
   "live band on stage concert", "stage lighting concert laser", "open air festival audience",
   "club night poster flyer", "beer tent festival visitors", "food truck festival"]),
 "safe_other": ("safe", 30, [
   "mountain landscape sunset", "portrait of an adult man smiling", "portrait of an adult woman", "cat sitting",
   "dog portrait", "product photograph headphones", "coffee cup table", "city skyline", "sneakers product", "bicycle street"]),
 "suggestive": ("review", 20, [
   "swimwear fashion show runway", "lingerie fashion show runway", "bikini model photoshoot adult",
   "lingerie advertisement vintage", "swimsuit competition adult contestants", "beachwear model 1950s",
   "Miss Universe swimsuit", "boudoir lingerie model"]),
 "nudity_artistic": ("review", 20, [
   "Birth of Venus Botticelli", "Venus de Milo", "David Michelangelo statue", "Olympia Manet", "La maja desnuda Goya",
   "La Grande Odalisque Ingres", "Discobolus Myron statue", "Venus of Urbino Titian", "Rokeby Venus Velazquez",
   "Capitoline Venus statue", "Apollo Belvedere", "Dying Gaul sculpture", "Venus Pudica statue", "Valpincon Bather Ingres",
   "Creation of Adam Michelangelo", "Perseus with the Head of Medusa Cellini", "Venus Anadyomene painting", "Hercules Farnese statue"]),
 "weapons": ("review", 30, [
   "AK-47 assault rifle", "hunting rifle", "pistol handgun", "revolver", "shotgun", "combat knife", "machete",
   "kitchen knife", "hunting knife", "submachine gun", "sniper rifle", "switchblade knife", "sword katana", "crossbow"]),
 "alcohol": ("review", 20, [
   "bottles of vodka", "whisky bottles", "beer bottles", "cocktail glass bar", "wine bottles", "shots tequila bar",
   "champagne bottle", "beer glass pint", "bartender cocktails"]),
 "drugs": ("reject", 20, [
   "cannabis plant", "marijuana joint", "hypodermic syringe", "pills tablets", "bong smoking pipe", "ecstasy pills MDMA",
   "cannabis buds", "drug paraphernalia", "cocaine powder", "hashish", "marijuana cigarette rolling", "LSD blotter", "crystal methamphetamine", "opium poppy", "psilocybin mushrooms", "ketamine"]),
 "smoking": ("review", 10, [
   "man smoking cigarette", "woman smoking cigarette", "cigarette pack", "hookah shisha", "cigar smoking", "cigarette butt ashtray"]),
 "gore_blood": ("review", 20, [
   "slaughterhouse", "fake blood makeup halloween", "gunshot wound", "wounded soldier battlefield photograph",
   "bandaged wound", "stage blood zombie costume", "bruise injury", "Civil War wounded soldiers photograph", "blood on hands",
   "bloody injury accident", "butcher meat carcass", "battlefield dead soldiers photograph", "scar wound stitches", "special effects makeup wound", "bloody nose"]),
 "hate_symbols": ("reject", 15, [
   "Nazi flag swastika", "Reichskriegsflagge", "Nazi rally Nuremberg", "Ku Klux Klan rally historical",
   "swastika flag", "Nazi propaganda poster", "Ku Klux Klan robes", "Confederate battle flag", "SS banner swastika", "Nazi Party rally banner", "Klan march Washington 1925"]),
 "offensive_gesture": ("review", 10, [
   "middle finger", "rude gesture finger", "flipping the bird", "giving the finger", "obscene gesture hand", "flipping off camera",
   "middle finger protest", "middle finger selfie", "fuck you gesture", "raised middle finger sign"]),
 "text_in_image": ("reject", 10, [
   "graffiti profanity insult", "poster with phone number", "advertisement phone number flyer",
   "insulting sign", "scam spam flyer", "rude sign text"]),
 # Added for the `underwear`, `swimwear`, `self_harm`, `minor` and `cannabis` concepts.
 "underwear": ("reject", 25, [
   "lingerie fashion show runway", "lingerie model photoshoot", "underwear model male", "boxer briefs model",
   "men's underwear advertisement", "bra and panties", "lingerie advertisement", "underwear fashion show"]),
 "swimwear": ("safe", 20, [
   "bikini beach woman", "swimsuit fashion show", "swimwear model photoshoot", "men swim trunks beach",
   "one-piece swimsuit woman", "beach volleyball players", "lifeguard on beach"]),
 "self_harm": ("reject", 15, [
   "self-harm scars", "self harm cutting scars arm", "self-inflicted wounds forearm", "cutting scars wrist",
   "self-harm awareness", "scars on forearm"]),
 # Everyday photos only. A nightlife platform keeps minors' photos out, so the
 # detector needs to see them; nothing undressed is ever fetched (BLOCK_BARE).
 "minors": ("reject", 80, [
   "children playing playground", "school children classroom", "kids birthday party", "child portrait smiling",
   "toddler playing toys", "boy playing football", "girl riding bicycle", "children at a festival",
   "family picnic children", "teenagers in school uniform", "kids concert audience", "children choir",
   "child portrait", "schoolboy portrait", "schoolgirl portrait", "teenage girl portrait", "teenage boy portrait",
   "kindergarten children", "children selfie", "father and son", "mother and daughter", "children dancing"]),
 "violence": ("reject", 25, [
   "street fight", "fistfight", "brawl", "riot police clash with protesters", "bar fight", "people fighting street",
   "police beating protester", "punch in the face fight", "football hooligans fighting"]),
 "cannabis": ("review", 15, [
   "cannabis plant", "marijuana joint", "cannabis buds", "hashish", "cannabis leaf", "cannabis dispensary"]),
}


def clean(s):
    return html.unescape(re.sub(r"<[^>]+>", "", s or "")).strip()


def ok_license(lic):
    l = (lic or "").lower()
    toks = re.split(r"[\s-]+", l)
    return any(k in l for k in ("cc by", "cc-by", "cc0", "public domain", "pd", "cc zero")) and "nc" not in toks and "nd" not in toks


def search(q, limit=30):
    p = {"action": "query", "format": "json", "generator": "search", "gsrsearch": f"{q} filetype:bitmap",
         "gsrnamespace": 6, "gsrlimit": limit, "prop": "imageinfo", "iiprop": "url|extmetadata|mime|size",
         "iiurlwidth": 1024, "iiextmetadatafilter": "LicenseShortName|Artist|ImageDescription|Categories|ObjectName"}
    for attempt in range(5):
        r = S.get(API, params=p, timeout=60)
        if r.status_code == 429:
            time.sleep(5 * (attempt + 1))
            continue
        r.raise_for_status()
        return list(r.json().get("query", {}).get("pages", {}).values())
    return []


def download(url):
    for attempt in range(5):
        r = S.get(url, timeout=60)
        if r.status_code == 429:
            time.sleep(5 * (attempt + 1))
            continue
        r.raise_for_status()
        return r.content
    raise RuntimeError("rate limited " + url)


manifest_path = os.path.join(OUT, "manifest.jsonl")
rows = []
if os.path.exists(manifest_path):
    rows = [json.loads(l) for l in open(manifest_path, encoding="utf-8")]
seen = {r["source_url"] for r in rows}

for cat, (expected, target, queries) in CATS.items():
    if ONLY and cat not in ONLY:
        continue
    have = sum(1 for r in rows if r["category"] == cat)
    os.makedirs(os.path.join(OUT, cat), exist_ok=True)
    cands = []
    for q in queries:
        try:
            pages = search(q)
        except Exception as e:
            print("search fail", q, e)
            continue
        time.sleep(0.4)
        pages.sort(key=lambda p: p.get("index", 0))
        cands.append([p for p in pages if p.get("imageinfo")])
    while have < target and any(cands):
        for lst in cands:
            if have >= target:
                break
            if not lst:
                continue
            pg = lst.pop(0)
            ii = pg["imageinfo"][0]
            md = ii.get("extmetadata", {})
            url = ii["descriptionurl"]
            if url in seen:
                continue
            title = pg["title"]
            text = " ".join([title, clean(md.get("ImageDescription", {}).get("value")),
                             clean(md.get("Categories", {}).get("value"))])
            lic = md.get("LicenseShortName", {}).get("value", "")
            if ii.get("mime") not in ("image/jpeg", "image/png") or not ok_license(lic):
                continue
            if min(ii.get("width", 0), ii.get("height", 0)) < 300:
                continue
            blocker = {"gore_blood": BLOCK_MINORS, "minors": BLOCK_BARE}.get(cat, BLOCK)
            if blocker.search(text):
                print("  skip block:", title)
                continue
            turl = ii.get("thumburl") or ii["url"]
            try:
                data = download(turl)
                time.sleep(0.5)
                im = Image.open(io.BytesIO(data)).convert("RGB")
                im.thumbnail((1024, 1024))
            except Exception as e:
                print("  dl fail", title, e)
                continue
            have += 1
            fn = f"{cat}_{have:03d}.jpg"
            im.save(os.path.join(OUT, cat, fn), "JPEG", quality=85)
            seen.add(url)
            row = {"path": f"{cat}/{fn}", "category": cat, "expected": expected, "source_url": url,
                   "license": lic, "author": clean(md.get("Artist", {}).get("value"))[:200], "title": title}
            rows.append(row)
            with open(manifest_path, "a", encoding="utf-8") as f:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(cat, have, "/", target, flush=True)
