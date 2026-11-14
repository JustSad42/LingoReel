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
      if (e.key === " " || e.key === "Enter") {
        e.preventDefault();
        if (!revealed) setRevealed(true);
        else grade(3);
      } else if (revealed && ["1", "2", "3", "4"].includes(e.key)) grade(+e.key);
      else if (e.key.toLowerCase() === "r") video.current?.play();
      else if (e.key.toLowerCase() === "h" && tts) hear(card.sentence);
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [card, revealed, grade, tts]);

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
      <div className="card flash">
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
        <div className="flash-sentence">
          <Sentence card={card} />
          {tts && (
            <button className="icon-btn small" title="Hear it read aloud (H)" onClick={() => hear(card.sentence)}>
              <Icon name="speak" size={17} />
            </button>
          )}
        </div>
        <div className="muted small-text">{card.media_title}</div>
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
                <button key={label} className={"answer a" + (i + 1)} onClick={() => grade(i + 1)}>
                  {label}
                  <span>{fmtInterval(card.preview[i])}</span>
                </button>
              ))}
            </div>
            <div className="muted small-text">Keys: 1 to 4 to answer, Space for Good, R to replay the clip</div>
          </div>
        ) : (
          <button className="primary" onClick={() => setRevealed(true)}>
            Show answer (Space)
          </button>
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
