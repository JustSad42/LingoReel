import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, Card, fmtInterval } from "../api";
import { useApp } from "../App";
import Icon from "../Icon";

const ANSWERS = ["Again", "Hard", "Good", "Easy"];

/** The sentence with the studied word emphasised. */
function Sentence({ card }: { card: Card }) {
  if (!card.word) return <>{card.sentence}</>;
  const i = card.sentence.toLowerCase().indexOf(card.word.toLowerCase());
  if (i < 0) return <>{card.sentence}</>;
  return (
    <>
      {card.sentence.slice(0, i)}
      <mark>{card.sentence.slice(i, i + card.word.length)}</mark>
      {card.sentence.slice(i + card.word.length)}
    </>
  );
}

type Style = "classic" | "cloze" | "listen";
/** Lowercase without accents, for forgiving answer checks. */
const plain = (t: string) => t.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase().trim();
type Check = { state: "right" | "accent" | "wrong"; typed: string };

/** The sentence with the studied word blanked out. */
function Cloze({ card, check }: { card: Card; check: Check | null }) {
  const i = card.sentence.toLowerCase().indexOf(card.word.toLowerCase());
  return (
    <>
      {card.sentence.slice(0, i)}
      {check ? (
        <mark className={"cloze-answer " + check.state}>{card.sentence.slice(i, i + card.word.length)}</mark>
      ) : (
        <span className="blank" style={{ width: Math.max(3, card.word.length) * 0.62 + "em" }} />
      )}
      {card.sentence.slice(i + card.word.length)}
    </>
  );
}

export default function Review() {
  const { settings, toast } = useApp();
  const lang = settings.targetLang;
  const [queue, setQueue] = useState<Card[]>([]);
  const [revealed, setRevealed] = useState(false);
  const [counts, setCounts] = useState({ due: 0, new: 0, total: 0, reviewed_today: 0 });
  const [loaded, setLoaded] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const [tts, setTts] = useState(false);
  const [ttsEn, setTtsEn] = useState(false);
  useEffect(() => {
    api.ttsReady(lang).then(setTts).catch(() => {});
    api.ttsReady("en").then(setTtsEn).catch(() => {});
  }, [lang]);
  const hear = async (text: string, voice = lang) => {
    try {
      video.current?.pause();
      await new Audio(convertFileSrc(await api.speak(voice, text, settings.ttsSlow))).play();
    } catch (e) {
      toast(String(e));
    }
  };

  const load = useCallback(async () => {
    setQueue(await api.cardsDue(lang, 50));
    setCounts(await api.cardsCounts(lang));
    setRevealed(false);
    setLoaded(true);
  }, [lang]);
  useEffect(() => {
    load();
  }, [load]);

  const card = queue[0];
  const [typed, setTyped] = useState("");
  const [check, setCheck] = useState<Check | null>(null);
  const clozeInput = useRef<HTMLInputElement>(null);
  const canCloze = !!card?.word && card.sentence.toLowerCase().includes(card.word.toLowerCase());
  const style: Style = (() => {
    if (!card) return "classic";
    const pick = settings.reviewStyle === "mixed" ? (["classic", "cloze", "listen"] as Style[])[card.id % 3] : (settings.reviewStyle as Style);
    if (pick === "cloze" && canCloze) return "cloze";
    if (pick === "listen" && (card.clip || tts)) return "listen";
    return "classic";
  })();
  useEffect(() => {
    setTyped("");
    setCheck(null);
    if (style === "cloze") window.setTimeout(() => clozeInput.current?.focus(), 50);
    // Listening cards without a clip start with the voice.
    if (style === "listen" && card && !card.clip && tts) hear(card.sentence);
  }, [card?.id, style]);
  const submitCloze = () => {
    if (!card || check) return;
    const want = card.word;
    const state = typed.trim().toLowerCase() === want.toLowerCase() ? "right" : plain(typed) === plain(want) ? "accent" : "wrong";
    setCheck({ state, typed: typed.trim() });
    setRevealed(true);
  };
  const suggested = check ? (check.state === "right" ? 3 : check.state === "accent" ? 2 : 1) : 3;
  const grade = useCallback(
    async (rating: number) => {
      if (!card) return;
      try {
        await api.cardGrade(card.id, rating);
      } catch (e) {
        toast(String(e));
      }
      setRevealed(false);
      setQueue((q) => q.slice(1));
      setCounts(await api.cardsCounts(lang));
    },
    [card, lang],
  );

  useEffect(() => {
    if (loaded && queue.length === 0 && counts.due + counts.new > 0) load();
  }, [queue.length, loaded]);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (!card) return;
      if ((e.target as HTMLElement).tagName === "INPUT") {
        if (e.key === "Enter") {
          e.preventDefault();
          submitCloze();
        }
        return;
      }
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        if (!revealed) setRevealed(true);
        else grade(suggested);
      } else if (revealed && ["1", "2", "3", "4"].includes(e.key)) grade(+e.key);
      else if (e.key.toLowerCase() === "r") {
        if (video.current) {
          video.current.currentTime = 0;
          video.current.play();
        } else if (tts) hear(card.sentence);
      }
      else if (e.key.toLowerCase() === "h" && tts) hear(card.sentence);
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [card, revealed, grade, tts, suggested, typed, check]);

  if (!loaded) return null;
  if (!card) {
    return (
      <div className="page narrow empty">
        <h1>Review</h1>
        {counts.total === 0 ? (
          <p className="muted">
            Nothing saved yet. While watching, click a word and choose “Save for review”, or press W to save the whole line.
            Each saved item keeps a short clip of the scene.
          </p>
        ) : (
          <p className="muted">
            All caught up. {counts.reviewed_today} reviewed in the last 24 hours, {counts.total} saved in total.
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="page narrow review">
      <div className="row">
        <h1>Review</h1>
        <div className="spacer" />
        <span className="muted">
          {counts.due} due · {counts.new} new
        </span>
      </div>
      <div className={"card flash style-" + style + (revealed ? " revealed" : "")}>
        {style !== "classic" && (
          <div className="flash-kind">
            {style === "cloze" ? "Fill in the missing word" : "Listen first, then check what was said"}
          </div>
        )}
        {card.clip ? (
          <video
            key={card.id}
            ref={video}
            src={convertFileSrc(card.clip)}
            poster={card.image ? convertFileSrc(card.image) : undefined}
            autoPlay
            controls
          />
        ) : (
          <div className="muted small-text">The scene clip is still being cut, or the video was unavailable.</div>
        )}
        {style === "listen" && !revealed && (
          <div className="listen-box">
            <button className="listen-play" onClick={() => (card.clip && video.current ? (video.current.currentTime = 0, video.current.play()) : hear(card.sentence))}>
              <Icon name="speak" size={30} />
            </button>
            <span className="muted">Play it again with R. What did you hear?</span>
          </div>
        )}
        <div className="flash-sentence">
          {style === "cloze" ? <Cloze card={card} check={check} /> : <Sentence card={card} />}
          {tts && (
            <button className="icon-btn small" title="Hear it read aloud (H)" onClick={() => hear(card.sentence)}>
              <Icon name="speak" size={17} />
            </button>
          )}
        </div>
        <div className="muted small-text">{card.media_title}</div>
        {style === "cloze" && !check && (
          <div className="cloze-entry">
            {card.translation && <div className="muted">{card.translation}</div>}
            <div className="row">
              <input ref={clozeInput} value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="Type the missing word" spellCheck={false} />
              <button className="primary" onClick={submitCloze}>
                Check
              </button>
              <button className="ghost" onClick={() => { setCheck({ state: "wrong", typed: "" }); setRevealed(true); }}>
                I don't know
              </button>
            </div>
          </div>
        )}
        {check && (
          <div className={"cloze-result " + check.state}>
            {check.state === "right"
              ? "Correct."
              : check.state === "accent"
                ? `Nearly: mind the accents (you wrote “${check.typed}”).`
                : check.typed
                  ? `You wrote “${check.typed}”.`
                  : "Here is the word."}
          </div>
        )}
        {revealed ? (
          <div className="flash-back">
            {card.word && (
              <div>
                <strong>{card.lemma && card.lemma !== card.word.toLowerCase() ? `${card.word} (${card.lemma})` : card.word}</strong>
                {card.definition ? ` — ${card.definition}` : ""}
              </div>
            )}
            {card.translation && (
              <div className="flash-translation">
                {card.translation}
                {ttsEn && (
                  <button className="icon-btn small" title="Hear the translation" onClick={() => hear(card.translation, "en")}>
                    <Icon name="speak" size={16} />
                  </button>
                )}
              </div>
            )}
            {!card.translation && !card.definition && <div className="muted">No translation was saved with this card.</div>}
            <div className="answers">
              {ANSWERS.map((label, i) => (
                <button key={label} className={"answer a" + (i + 1) + (i + 1 === suggested ? " suggested" : "")} onClick={() => grade(i + 1)}>
                  {label}
                  <span>{fmtInterval(card.preview[i])}</span>
                </button>
              ))}
            </div>
            <div className="muted small-text">Keys: 1 to 4 to answer, Space for {ANSWERS[suggested - 1]}, R to replay the clip</div>
          </div>
        ) : (
          style !== "cloze" && (
            <button className="primary" onClick={() => setRevealed(true)}>
              Show answer (Space)
            </button>
          )
        )}
      </div>
      <div className="row end">
        <button
          className="ghost small"
          onClick={async () => {
            await api.cardDelete(card.id);
            setQueue((q) => q.slice(1));
          }}
        >
          Delete this card
        </button>
      </div>
    </div>
  );
}
