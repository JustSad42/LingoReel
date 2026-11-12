import { useEffect, useMemo, useState } from "react";
import { api, Card, fmtInterval, LANGS, STATUS } from "../api";
import { useApp } from "../App";

export default function Words() {
  const { settings, words, setWords, toast } = useApp();
  const lang = settings.targetLang;
  const [tab, setTab] = useState<"words" | "cards">("words");
  const [filter, setFilter] = useState(1);
  const [query, setQuery] = useState("");
  const [cards, setCards] = useState<Card[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.cardsList(lang).then(setCards);
  }, [lang]);

  const counts = useMemo(() => {
    const c = [0, 0, 0, 0];
    for (const st of words.values()) c[st]++;
    return c;
  }, [words]);

  const list = useMemo(() => {
    const q = query.trim().toLowerCase();
    return [...words.entries()]
      .filter(([l, st]) => st === filter && (!q || l.includes(q)))
      .sort((a, b) => a[0].localeCompare(b[0]))
      .slice(0, 500);
  }, [words, filter, query]);

  const markTop = async (n: number) => {
    setBusy(true);
    try {
      const top = await api.topLemmas(lang, n);
      const fresh = top.filter((l) => (words.get(l) ?? 0) === 0);
      setWords(fresh, 2);
      toast(`Marked ${fresh.length} common words as known`);
    } catch (e) {
      toast(String(e));
    }
    setBusy(false);
  };

  const now = Date.now() / 1000;
  return (
    <div className="page">
      <h1>Vocabulary · {LANGS[lang] || lang}</h1>
      <div className="stat-row">
        <div className="stat">
          <strong>{counts[2]}</strong>
          <span>known</span>
        </div>
        <div className="stat">
          <strong>{counts[1]}</strong>
          <span>learning</span>
        </div>
        <div className="stat">
          <strong>{cards.length}</strong>
          <span>saved for review</span>
        </div>
      </div>

      <section className="card">
        <h2>Already know some {LANGS[lang] || lang}?</h2>
        <p className="muted">
          Mark the most common words as known so subtitles only highlight what is new to you, and difficulty scores reflect
          your level. You can change any word later by clicking it.
        </p>
        <div className="row">
          {[100, 300, 500, 1000, 2000].map((n) => (
            <button key={n} className="small" disabled={busy} onClick={() => markTop(n)}>
              Top {n}
            </button>
          ))}
        </div>
      </section>

      <div className="row tabs-row">
        <button className={tab === "words" ? "tab active" : "tab"} onClick={() => setTab("words")}>
          Tracked words
        </button>
        <button className={tab === "cards" ? "tab active" : "tab"} onClick={() => setTab("cards")}>
          Saved for review
        </button>
      </div>

      {tab === "words" && (
        <>
          <div className="row">
            {[1, 2, 3].map((st) => (
              <button key={st} className={"small " + (filter === st ? "primary" : "ghost")} onClick={() => setFilter(st)}>
                {STATUS[st]} ({counts[st]})
              </button>
            ))}
            <div className="spacer" />
            <input placeholder="Filter…" value={query} onChange={(e) => setQuery(e.target.value)} />
          </div>
          <div className="word-grid">
            {list.map(([lemma, st]) => (
              <div key={lemma} className="word-chip">
                <span>{lemma}</span>
                <select value={st} onChange={(e) => setWords([lemma], +e.target.value)}>
                  {STATUS.map((label, i) => (
                    <option key={i} value={i}>
                      {label}
                    </option>
                  ))}
                </select>
              </div>
            ))}
            {list.length === 0 && <div className="muted">No words here yet.</div>}
          </div>
        </>
      )}

      {tab === "cards" && (
        <div className="card-list">
          {cards.map((c) => (
            <div key={c.id} className="card-row">
              <div>
                <strong>{c.word || "Whole line"}</strong> <span className="muted">{c.definition}</span>
                <div>{c.sentence}</div>
                <div className="muted small-text">
                  {c.media_title} · {c.state === 0 ? "new" : c.due <= now ? "due now" : "next in " + fmtInterval(c.due - now)}
                </div>
              </div>
              <button
                className="ghost small"
                onClick={async () => {
                  await api.cardDelete(c.id);
                  setCards((list) => list.filter((x) => x.id !== c.id));
                }}
              >
                Delete
              </button>
            </div>
          ))}
          {cards.length === 0 && <div className="muted">Nothing saved yet.</div>}
        </div>
      )}
    </div>
  );
}
