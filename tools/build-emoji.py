#!/usr/bin/env python3
"""Build data/emoji/emoji.tsv from Unicode emoji-test.txt and CLDR annotations.

Sources (Unicode License v3, see data/emoji/LICENSE-UNICODE):
  https://unicode.org/Public/emoji/<ver>/emoji-test.txt
  https://github.com/unicode-org/cldr-json  (cldr-annotations-full, cldr-annotations-derived-full)

Output columns: emoji, group, then one keyword column per language
(keywords separated by '|', first entry is the CLDR short name).
Only fully-qualified emoji are kept; skin tone variants are folded into
their base emoji (column 'tones' lists the variants).
"""
import hashlib, json, sys, urllib.request, pathlib

EMOJI_VER = "16.0"
CLDR_VER = "46.0.0"
LANGS = ["en", "de", "ru"]
ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "data" / "emoji" / "emoji.tsv"
CACHE = ROOT / ".devtools" / "emoji-cache"
SHA256 = {
    "emoji-test-16.0.txt": "24f0c534e86cf142e2496953e8f0e46a3e702392911eddcd29c6cced85139697",
    "annotations-en-46.0.0.json": "2405d3f2e4c5812a4faf93d965630246269e0177ce907d863c7d6bb197e957c1",
    "annotations-de-46.0.0.json": "1071d9ccbcad11b9c02f05fa5a2d5f42feddb0cb177d20132e40ac988d14c3af",
    "annotations-ru-46.0.0.json": "b4f934ae432c2c8bb6bf4126962dbb02f4810d93bd681c6da018ed9b60a6c170",
    "annotationsDerived-en-46.0.0.json": "603b94176ee294ba2454a0c85a2835082ff4812cbeb5bb4e3ea78664f28ba16d",
    "annotationsDerived-de-46.0.0.json": "80f25e89b409cde9ab5bc028d02dc1746f3a9da7aba7946830495edc431b93a5",
    "annotationsDerived-ru-46.0.0.json": "fe62f4476844b9b8889040db8bbe1236c82549153f34d6f11bc571554d8f3ce0",
}

def fetch(url, name):
    CACHE.mkdir(parents=True, exist_ok=True)
    p = CACHE / name
    if p.exists():
        data = p.read_bytes()
    else:
        with urllib.request.urlopen(url, timeout=60) as r:
            data = r.read()
    got = hashlib.sha256(data).hexdigest()
    if got != SHA256[name]:
        sys.exit(f"checksum mismatch for {name}: expected {SHA256[name]}, got {got}")
    p.write_bytes(data)
    return data.decode("utf-8")

TONES = {chr(c) for c in range(0x1F3FB, 0x1F400)}

def main():
    test = fetch(f"https://unicode.org/Public/emoji/{EMOJI_VER}/emoji-test.txt", f"emoji-test-{EMOJI_VER}.txt")
    ann = {}
    for lang in LANGS:
        for kind in ["annotations", "annotationsDerived"]:
            pkg = "cldr-annotations-full" if kind == "annotations" else "cldr-annotations-derived-full"
            url = f"https://raw.githubusercontent.com/unicode-org/cldr-json/{CLDR_VER}/cldr-json/{pkg}/{kind}/{lang}/annotations.json"
            data = json.loads(fetch(url, f"{kind}-{lang}-{CLDR_VER}.json"))
            entries = data[kind]["annotations"] if kind in data else data["annotationsDerived"]["annotations"]
            for e, v in entries.items():
                names = v.get("tts", []) + v.get("default", [])
                d = ann.setdefault(e.replace("️", ""), {})
                cur = d.setdefault(lang, [])
                for n in names:
                    if n not in cur:
                        cur.append(n)
    rows, base_index = [], {}
    group = ""
    for line in test.splitlines():
        if line.startswith("# group:"):
            group = line.split(":", 1)[1].strip()
            continue
        if not line or line.startswith("#"):
            continue
        cps, rest = line.split(";", 1)
        if "fully-qualified" not in rest.split("#")[0]:
            continue
        if group == "Component":
            continue
        e = "".join(chr(int(c, 16)) for c in cps.split())
        base = "".join(ch for ch in e if ch not in TONES)
        if base != e and base in base_index:
            rows[base_index[base]]["tones"].append(e)
            continue
        key = e.replace("️", "")
        kw = ann.get(key, {})
        base_index[e] = len(rows)
        rows.append({"e": e, "group": group, "kw": kw, "tones": []})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("# emoji\tgroup\ttones\t" + "\t".join(LANGS) + f"\t# Unicode emoji {EMOJI_VER}, CLDR {CLDR_VER}\n")
        for r in rows:
            cols = [r["e"], r["group"], " ".join(r["tones"])]
            for lang in LANGS:
                cols.append("|".join(k.replace("\t", " ").replace("|", "/") for k in r["kw"].get(lang, [])))
            f.write("\t".join(cols) + "\n")
    print(f"wrote {len(rows)} emoji to {OUT}")

if __name__ == "__main__":
    sys.exit(main())
