import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, Component } from "../api";
import { useApp } from "../App";

type Progress = { id: string; done: number; total: number; stage: string };

export default function Setup({ onDone }: { onDone: () => void }) {
  const { toast, save } = useApp();
  const [comps, setComps] = useState<Component[]>([]);
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const queue = useRef<Promise<void>>(Promise.resolve());

  const refresh = () => api.components().then(setComps);
  useEffect(() => {
    refresh();
    const un = listen<Progress>("component-progress", (e) =>
      setProgress((p) => ({ ...p, [e.payload.id]: e.payload })),
    );
    return () => {
      un.then((f) => f());
    };
  }, []);

  const install = (ids: string[]) => {
    setBusy((b) => new Set([...b, ...ids]));
    for (const id of ids) {
      queue.current = queue.current.then(async () => {
        try {
          await api.install(id);
        } catch (e) {
          toast(`Could not install ${id}: ${e}`);
        }
        setBusy((b) => {
          const n = new Set(b);
          n.delete(id);
          return n;
        });
        await refresh();
      });
    }
  };

  const remove = async (id: string) => {
    await api.remove(id);
    refresh();
  };

  const missing = (pred: (c: Component) => boolean) => comps.filter((c) => !c.installed && pred(c)).map((c) => c.id);
  const requiredMissing = missing((c) => c.required);
  const speech = ["whisper", "whisper-model"];
  const llm = ["llama", "llm-model"];
  const group = (title: string, blurb: string, list: Component[], action?: JSX.Element) => (
    <section className="card">
      <div className="row">
        <div>
          <h2>{title}</h2>
          <p className="muted">{blurb}</p>
        </div>
        <div className="spacer" />
        {action}
      </div>
      {list.map((c) => {
        const p = progress[c.id];
        const working = busy.has(c.id);
        const pct = p && p.total > 0 ? Math.round((p.done / p.total) * 100) : 0;
        return (
          <div className="comp" key={c.id}>
            <div className="comp-text">
              <strong>{c.name}</strong>
              <span className="muted">
                {c.about} {c.note}
              </span>
            </div>
            <div className="comp-side">
              {working ? (
                <div className="bar" title={p?.stage === "unpack" ? "Unpacking" : `${pct}%`}>
                  <div style={{ width: (p?.stage === "unpack" ? 100 : pct) + "%" }} />
                  <span>{p?.stage === "unpack" ? "Unpacking…" : p ? `${pct}%` : "Waiting…"}</span>
                </div>
              ) : c.installed ? (
                <>
                  <span className="ok">Installed</span>
                  {!c.note && (
                    <button className="ghost small" onClick={() => remove(c.id)}>
                      Remove
                    </button>
                  )}
                </>
              ) : (
                <button className="small" onClick={() => install([c.id])}>
                  Download ({c.size_mb >= 1000 ? (c.size_mb / 1000).toFixed(1) + " GB" : c.size_mb + " MB"})
                </button>
              )}
            </div>
          </div>
        );
      })}
    </section>
  );

  const allBtn = (ids: string[], label: string) =>
    ids.length > 0 ? (
      <button className="primary" disabled={ids.some((i) => busy.has(i))} onClick={() => install(ids)}>
        {label}
      </button>
    ) : undefined;

  return (
    <div className="page narrow">
      <h1>Setup</h1>
      <p className="muted">
        LingoReel downloads its engines and data once, then works offline. Everything is stored on this PC.
      </p>
      {group(
        "Required",
        "The player, subtitle reader and dictionary.",
        comps.filter((c) => c.required),
        allBtn(requiredMissing, "Download all"),
      )}
      {group(
        "Word-by-word timing",
        "Listens to each video once so every word lights up exactly when it is spoken, fixes badly timed subtitles, and can write subtitles when a video has none.",
        comps.filter((c) => speech.includes(c.id)),
        allBtn(missing((c) => speech.includes(c.id)), "Download both"),
      )}
      {group(
        "Phrase explanations (optional)",
        "A small language model that runs on your own graphics card to explain idioms and grammar. No internet or account needed once downloaded.",
        comps.filter((c) => llm.includes(c.id)),
        allBtn(missing((c) => llm.includes(c.id)), "Download both"),
      )}
      <div className="row end">
        <button
          className="primary"
          disabled={requiredMissing.length > 0}
          onClick={() => {
            save({ setupDone: true });
            onDone();
          }}
        >
          {requiredMissing.length > 0 ? "Download the required items to continue" : "Go to library"}
        </button>
      </div>
    </div>
  );
}
