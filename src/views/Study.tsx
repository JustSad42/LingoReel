import { useEffect, useState } from "react";
import { api, Conjugation, TenseExample, TENSE_NAMES, tenseNames } from "../api";
import { CONJ_CORE, CONJ_PERSONS, CONJ_ROWS, grammarNote, ROW_TENSE } from "../grammar";
import Icon from "../Icon";

/** A centred window over the player or a page. */
export function Modal({ title, sub, onClose, children, wide }: { title: React.ReactNode; sub?: React.ReactNode; onClose: () => void; children: React.ReactNode; wide?: boolean }) {
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [onClose]);
  return (
    <div className="modal-scrim" onClick={(e) => { e.stopPropagation(); onClose(); }}>
      <div className={"modal" + (wide ? " wide" : "")} onClick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
        <div className="modal-head">
          <div>
            <h2>{title}</h2>
            {sub && <div className="muted">{sub}</div>}
          </div>
          <div className="spacer" />
          <button className="icon-btn small" onClick={onClose} title="Close (Esc)">
            <Icon name="close" size={16} />
          </button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}

/** What a tense is for, how it is built, and lines from the learner's own videos that use it. */
export function GrammarModal({ lang, code, onClose, onPlay }: { lang: string; code: string; onClose: () => void; onPlay?: (ex: TenseExample) => void }) {
  const [examples, setExamples] = useState<TenseExample[] | null>(null);
  useEffect(() => {
    setExamples(null);
    api.tenseExamples(lang, code).then(setExamples).catch(() => setExamples([]));
  }, [lang, code]);
  const name = (tenseNames(lang) || TENSE_NAMES)[code] || code;
  const note = grammarNote(lang, code);
  const mark = (text: string, word: string) => {
    const i = word ? text.indexOf(word) : -1;
    if (i < 0) return text;
    return (
      <>
        {text.slice(0, i)}
        <b className={"g-" + code}>{word}</b>
        {text.slice(i + word.length)}
      </>
    );
  };
  return (
    <Modal title={<span className={"tense-title g-" + code}><i /> {name}</span>} sub="Grammar note" onClose={onClose}>
      {note ? (
        <>
          <section className="note">
            <h4>When it is used</h4>
            <p>{note.use}</p>
          </section>
          <section className="note">
            <h4>How it is formed</h4>
            <p>{note.form}</p>
          </section>
          <section className="note">
            <h4>Examples</h4>
            {note.example.map(([a, b], i) => (
              <div key={i} className="example">
                <div>{a}</div>
                <div className="muted">{b}</div>
              </div>
            ))}
          </section>
        </>
      ) : (
        <p className="muted">Lines from your videos that use this form are shown below.</p>
      )}
      <section className="note">
        <h4>From your own videos</h4>
        {examples === null && <div className="muted">Looking through your prepared videos…</div>}
        {examples && examples.length === 0 && <div className="muted">None yet. Prepare a few more videos and examples will appear here.</div>}
        {examples?.map((ex, i) => (
          <div key={i} className="example mine">
            <div>{mark(ex.text, ex.word)}</div>
            {ex.en && <div className="muted">{ex.en}</div>}
            <div className="example-src">
              <span>{ex.title}</span>
              {onPlay && (
                <button className="ghost small" onClick={() => onPlay(ex)}>
                  <Icon name="play" size={13} /> Play
                </button>
              )}
            </div>
          </div>
        ))}
      </section>
    </Modal>
  );
}

/** Conjugation table for a verb, with the form that was clicked highlighted. */
export function ConjugationModal({ lang, lemma, form, onClose, onTense }: { lang: string; lemma: string; form?: string; onClose: () => void; onTense?: (code: string) => void }) {
  const [c, setC] = useState<Conjugation | null | undefined>(undefined);
  const [all, setAll] = useState(false);
  useEffect(() => {
    api.conjugate(lang, lemma).then(setC).catch(() => setC(null));
  }, [lang, lemma]);
  const persons = CONJ_PERSONS[lang] || ["1st sing.", "2nd sing.", "3rd sing.", "1st pl.", "2nd pl.", "3rd pl."];
  const labels = CONJ_ROWS[lang] || {};
  const hit = (form || "").toLowerCase();
  const rows = c ? c.rows.filter((r) => all || CONJ_CORE.includes(r.code)) : [];
  const hidden = c ? c.rows.length - c.rows.filter((r) => CONJ_CORE.includes(r.code)).length : 0;
  return (
    <Modal
      title={lemma}
      sub={c ? [c.participle && `participle ${c.participle}`, c.gerund && `gerund ${c.gerund}`].filter(Boolean).join(" · ") : "Conjugation"}
      onClose={onClose}
      wide
    >
      {c === undefined && <div className="muted">Building the table…</div>}
      {c === null && <div className="muted">The dictionary has no conjugation for this word.</div>}
      {c && (
        <>
          <div className="conj-grid">
            {rows.map((r) => (
              <div key={r.code} className={"conj-card" + (r.cells.some((x) => x.toLowerCase() === hit || x.toLowerCase().endsWith(" " + hit)) ? " has-hit" : "")}>
                <div className={"conj-title g-" + r.code}>
                  <i />
                  <span>{labels[r.code] || r.code}</span>
                  {onTense && ROW_TENSE[r.code] && grammarNote(lang, ROW_TENSE[r.code]) && (
                    <button className="ghost small" title="Grammar note" onClick={() => onTense(ROW_TENSE[r.code])}>?</button>
                  )}
                </div>
                <table>
                  <tbody>
                    {r.cells.map((cell, i) =>
                      cell ? (
                        <tr key={i} className={cell.toLowerCase() === hit || cell.toLowerCase().endsWith(" " + hit) ? "hit" : ""}>
                          <td className="muted">{r.code === "imper" ? "" : persons[i]}</td>
                          <td>{cell}</td>
                        </tr>
                      ) : null,
                    )}
                  </tbody>
                </table>
              </div>
            ))}
          </div>
          {hidden > 0 && (
            <button className="ghost small" onClick={() => setAll(!all)}>
              {all ? "Show the main tenses only" : `Show ${hidden} more (literary and imperative forms)`}
            </button>
          )}
        </>
      )}
    </Modal>
  );
}
