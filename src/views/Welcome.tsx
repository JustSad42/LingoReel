import { useState } from "react";
import { Release } from "../changelog";
import Icon from "../Icon";
import { Modal } from "./Study";

const STEPS: { icon: string; title: string; body: string; keys?: [string, string][] }[] = [
  {
    icon: "film",
    title: "Learn from what you already watch",
    body: "LingoReel plays your own films and series, from Plex or from folders, with the language you are learning and English shown together. Films in English work too: you read the subtitles in your language with the English underneath.",
  },
  {
    icon: "download",
    title: "First, a one-time download",
    body: "Setup fetches the player, a dictionary and (if you want) speech recognition, a voice and a local model. After that everything runs on this PC, offline. Choose Light setup for a smaller start.",
  },
  {
    icon: "queue",
    title: "Prepare videos ahead",
    body: "Each video is prepared once: every word is matched to the audio and looked up. Queue whole seasons from the Library and they get ready in the background.",
  },
  {
    icon: "sparkles",
    title: "Click any word",
    body: "Words are underlined orange when new and yellow while you learn them. Click one for its meaning, grammar, conjugation and related words. Dotted lines mark set phrases.",
    keys: [
      ["A / D", "previous / next line"],
      ["S", "replay the line"],
      ["Q", "back, with the English"],
      ["B", "loop the line"],
      ["P", "auto-pause"],
      ["W", "save the line"],
    ],
  },
  {
    icon: "review",
    title: "Review a few minutes a day",
    body: "Saved words and lines come back with a clip of the scene, on a schedule that spaces them out as you learn them. Cards can ask you to fill in the missing word or to listen before you read.",
  },
];

/** A few screens that explain the app on first start. */
export function Tour({ onClose }: { onClose: () => void }) {
  const [i, setI] = useState(0);
  const step = STEPS[i];
  const last = i === STEPS.length - 1;
  return (
    <Modal title="Welcome to LingoReel" sub={`${i + 1} of ${STEPS.length}`} onClose={onClose}>
      <div className="tour-step" key={i}>
        <div className="tour-icon">
          <Icon name={step.icon} size={30} />
        </div>
        <h3>{step.title}</h3>
        <p>{step.body}</p>
        {step.keys && (
          <div className="tour-keys">
            {step.keys.map(([k, v]) => (
              <div key={k}>
                <kbd>{k}</kbd> <span className="muted">{v}</span>
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="tour-dots">
        {STEPS.map((_, j) => (
          <button key={j} className={j === i ? "on" : ""} onClick={() => setI(j)} aria-label={`Step ${j + 1}`} />
        ))}
      </div>
      <div className="row end">
        <button className="ghost" onClick={onClose}>
          Skip
        </button>
        {i > 0 && <button onClick={() => setI(i - 1)}>Back</button>}
        <button className="primary" onClick={() => (last ? onClose() : setI(i + 1))}>
          {last ? "Get started" : "Next"}
        </button>
      </div>
    </Modal>
  );
}

/** What is new since the version the learner used last. */
export function Changes({ releases, onClose }: { releases: Release[]; onClose: () => void }) {
  return (
    <Modal title="What's new" sub={releases.length ? `LingoReel ${releases[0].version}` : undefined} onClose={onClose}>
      {releases.map((r) => (
        <section key={r.version} className="changes">
          <h4>
            {r.version} · {r.title}
          </h4>
          {r.items.map(([t, d]) => (
            <div key={t} className="change">
              <Icon name="check" size={15} />
              <div>
                <strong>{t}</strong>
                <span className="muted"> {d}</span>
              </div>
            </div>
          ))}
        </section>
      ))}
      <div className="row end">
        <button className="primary" onClick={onClose}>
          Got it
        </button>
      </div>
    </Modal>
  );
}
