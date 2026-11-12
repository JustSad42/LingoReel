"""Build LingoReel's offline dictionary for one language.

Input:  kaikki.org Wiktionary extract (JSONL, gzipped) + a frequency list
        ("word count" per line, most frequent first).
Output: dict-<lang>.sqlite(.gz), uploaded as a GitHub release asset that the
        app downloads once on first run.

Usage:  python tools/build_dict.py fr French
Data is from Wiktionary (CC BY-SA 4.0) via kaikki.org and from
hermitdave/FrequencyWords (CC BY-SA 4.0, OpenSubtitles-derived).
"""
import gzip
import json
import os
import shutil
import sqlite3
import sys
import unicodedata
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(HERE, ".cache")

SKIP_POS = {"character", "symbol", "punct"}
SKIP_FORM_TAGS = {"table-tags", "inflection-template", "class"}
KEEP_SENSE_TAGS = {
    "informal", "colloquial", "slang", "vulgar", "formal", "literary", "archaic",
    "dated", "figuratively", "transitive", "intransitive", "reflexive",
    "pronominal", "impersonal", "offensive", "derogatory", "familiar",
    "Quebec", "Canada", "Belgium", "Switzerland", "Louisiana",
}
MAX_GLOSSES = 8


def norm(s: str) -> str:
    s = unicodedata.normalize("NFC", s).replace("’", "'").replace("ʼ", "'")
    return s.lower().strip()


def fetch(url: str, dest: str):
    if os.path.exists(dest):
        return
    print("downloading", url)
    with urllib.request.urlopen(url) as r, open(dest + ".part", "wb") as f:
        shutil.copyfileobj(r, f)
    os.replace(dest + ".part", dest)


def gender_of(d) -> str:
    for h in d.get("head_templates", []) or []:
        g = (h.get("args") or {}).get("1", "")
        if h.get("name", "").endswith("noun") and g:
            g = g.split("-")[0]
            if g in ("m", "f", "mf", "mfbysense"):
                return "m/f" if g.startswith("mf") else g
    tags = set()
    for s in d.get("senses", []) or []:
        tags.update(s.get("tags") or [])
    if d.get("pos") in ("noun", "name"):
        m, f = "masculine" in tags, "feminine" in tags
        if m and f:
            return "m/f"
        if m:
            return "m"
        if f:
            return "f"
    return ""


def main():
    lang, lang_name = sys.argv[1], sys.argv[2]
    os.makedirs(CACHE, exist_ok=True)
    src = os.path.join(CACHE, f"{lang}.jsonl.gz")
    freq_src = os.path.join(CACHE, f"{lang}_50k.txt")
    fetch(f"https://kaikki.org/dictionary/{lang_name}/kaikki.org-dictionary-{lang_name}.jsonl.gz", src)
    fetch(f"https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/{lang}/{lang}_50k.txt", freq_src)

    out = os.path.join(CACHE, f"dict-{lang}.sqlite")
    if os.path.exists(out):
        os.remove(out)
    db = sqlite3.connect(out)
    db.executescript(
        """
        CREATE TABLE entry(id INTEGER PRIMARY KEY, word TEXT, norm TEXT, pos TEXT,
                           gender TEXT, ipa TEXT, glosses TEXT);
        CREATE TABLE form(norm TEXT, lemma TEXT, tags TEXT);
        CREATE TABLE freq(norm TEXT PRIMARY KEY, rank INTEGER);
        CREATE TABLE meta(k TEXT PRIMARY KEY, v TEXT);
        """
    )
    entries, forms = [], set()
    with gzip.open(src, "rt", encoding="utf-8") as f:
        for line in f:
            d = json.loads(line)
            word, pos = d.get("word"), d.get("pos", "")
            if not word or pos in SKIP_POS or len(word) > 60:
                continue
            n = norm(word)
            glosses = []
            for s in d.get("senses", []) or []:
                tags = s.get("tags") or []
                targets = s.get("form_of") or s.get("alt_of")
                if targets:
                    label = " ".join(t for t in tags if t not in ("form-of", "alt-of"))
                    if s.get("alt_of") and not label:
                        label = "alternative form"
                    for t in targets:
                        if t.get("word"):
                            forms.add((n, t["word"], label))
                    continue
                g = s.get("glosses") or []
                if not g or "no-gloss" in tags:
                    continue
                text = g[-1].strip()
                keep = [t for t in tags if t in KEEP_SENSE_TAGS]
                if keep:
                    text = "(" + ", ".join(keep) + ") " + text
                if text not in glosses:
                    glosses.append(text)
            for fm in d.get("forms", []) or []:
                ftags = fm.get("tags") or []
                form = fm.get("form") or ""
                if not form or form == "-" or SKIP_FORM_TAGS & set(ftags):
                    continue
                if " " in form and " " not in word:
                    continue  # compound tenses like "avoir aimé"
                fn = norm(form)
                if fn != n:
                    forms.add((fn, word, " ".join(ftags)))
            if glosses:
                ipa = next((x["ipa"] for x in d.get("sounds", []) or [] if x.get("ipa")), "")
                entries.append((word, n, pos, gender_of(d), ipa,
                                json.dumps(glosses[:MAX_GLOSSES], ensure_ascii=False)))

    lemmas = {e[0] for e in entries}
    forms = [f for f in forms if f[1] in lemmas]
    db.executemany("INSERT INTO entry(word,norm,pos,gender,ipa,glosses) VALUES(?,?,?,?,?,?)", entries)
    db.executemany("INSERT INTO form VALUES(?,?,?)", forms)

    rank, seen = 0, set()
    with open(freq_src, encoding="utf-8") as f:
        for line in f:
            w = norm(line.split(" ")[0])
            if w and w not in seen:
                seen.add(w)
                rank += 1
                db.execute("INSERT INTO freq VALUES(?,?)", (w, rank))
    db.executemany("INSERT INTO meta VALUES(?,?)", [
        ("lang", lang), ("version", "1"),
        ("source", "Wiktionary via kaikki.org (CC BY-SA 4.0); FrequencyWords (CC BY-SA 4.0)"),
    ])
    db.executescript("CREATE INDEX entry_norm ON entry(norm); CREATE INDEX form_norm ON form(norm);")
    db.commit()
    db.execute("VACUUM")
    db.close()
    with open(out, "rb") as fi, gzip.open(out + ".gz", "wb", compresslevel=9) as fo:
        shutil.copyfileobj(fi, fo)
    print(f"entries={len(entries)} forms={len(forms)} freq={rank}")
    print(f"sqlite={os.path.getsize(out)//1_000_000}MB gz={os.path.getsize(out + '.gz')//1_000_000}MB")


if __name__ == "__main__":
    main()
