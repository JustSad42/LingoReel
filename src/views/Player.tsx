import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, Cue, Entry, fmtTime, freqLabel, LANGS, Lookup, MediaReq, MediaStats, Prepared, STATUS, SubTrack } from "../api";
import { useApp } from "../App";

const PANEL_W = 420;
const TEXT_CODECS = ["subrip", "srt", "ass", "ssa", "mov_text", "tx3g", "webvtt", "vtt", "text"];
const SPEEDS = [0.5, 0.65, 0.8, 0.9, 1, 1.15, 1.3];

type Popup = {
  cue: number;
  from: number;
  to: number;
  x: number;
  lookup?: Lookup;
  phrase?: Entry[];
  explain?: string;
  explaining?: boolean;
};
type Shadow = {
  phase: "listen" | "record" | "scoring" | "result";
  cue: number;
  result?: { heard: string; words: [string, boolean][]; score: number };
  audioUrl?: string;
  error?: string;
};

/** Index of the cue being spoken at time t, or -1. */
function cueAt(cues: Cue[], t: number): number {
  let lo = 0;
  let hi = cues.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid].end <= t) lo = mid + 1;
    else if (cues[mid].start > t) hi = mid - 1;
    else return mid;
  }
  return -1;
}
/** Index of the last cue that started at or before t, or -1. */
function lastStarted(cues: Cue[], t: number): number {
  let lo = 0;
  let hi = cues.length - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid].start <= t) {
      ans = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return ans;
}

function trackLabel(t: SubTrack): string {
  const lang = LANGS[t.lang.slice(0, 2)] || t.lang || "Unknown language";
  const readable = TEXT_CODECS.includes(t.codec.toLowerCase());
  return `${lang}${t.title ? " · " + t.title : ""} · ${t.index != null ? "in the video file" : "separate file"}${
    readable ? "" : " (picture-based, cannot be read)"
  }`;
}

export default function Player({ media, onExit }: { media: MediaReq; onExit: () => void }) {
  const { settings, save, words, setWords, toast } = useApp();
  const lang = settings.targetLang;

  const [phase, setPhase] = useState<"loading" | "choose" | "preparing" | "playing">("loading");
  const [prepared, setPrepared] = useState<Prepared | null>(null);
  const [prep, setPrep] = useState({ stage: "", pct: 0, note: "" });
  const [prepError, setPrepError] = useState("");
  const [targetSub, setTargetSub] = useState("auto");
  const [nativeSub, setNativeSub] = useState("auto");
  const [canListen, setCanListen] = useState(false);
  const [align, setAlign] = useState(settings.align);
  const [llm, setLlm] = useState(false);

  const [paused, setPaused] = useState(false);
  const [duration, setDuration] = useState(media.duration);
  const [uiTime, setUiTime] = useState(0);
  const [baseSpeed, setBaseSpeed] = useState(1);
  const [cueIdx, setCueIdx] = useState(-1);
  const [audioTracks, setAudioTracks] = useState<any[]>([]);
  const [aid, setAid] = useState<number | null>(null);
  const [panel, setPanel] = useState<"none" | "transcript" | "words">("none");
  const [popup, setPopup] = useState<Popup | null>(null);
  const [controls, setControls] = useState(true);
  const [reveal, setReveal] = useState(-1);
  const [translations, setTranslations] = useState<Record<number, string>>({});
  const [shadow, setShadow] = useState<Shadow | null>(null);
  const [stats, setStats] = useState<MediaStats | null>(null);
  const [filter, setFilter] = useState("");
  const [fullscreen, setFullscreen] = useState(false);

  const clock = useRef({ t: 0, stamp: performance.now() });
  const now = useRef(0);
  const pausedRef = useRef(false);
  const speedRef = useRef(1);
  const cuesRef = useRef<Cue[]>([]);
  const idxRef = useRef(-1);
  const holdIdx = useRef(-1);
  const autoPausedFor = useRef(-1);
  const stopAt = useRef<{ t: number; then: () => void } | null>(null);
  const lineRef = useRef<HTMLDivElement>(null);
  const activeRow = useRef<HTMLDivElement>(null);
  const hoverPaused = useRef(false);
  const popupPaused = useRef(false);
  const hideTimer = useRef<number>();
  const recorder = useRef<MediaRecorder | null>(null);
  const live = useRef({ settings, words, baseSpeed, duration });
  live.current = { settings, words, baseSpeed, duration };

  const cues = prepared?.cues ?? [];
  cuesRef.current = cues;

  // ---------- start-up ----------
  const startPlayback = useCallback(
    async (p: Prepared) => {
      setPrepared(p);
      setPhase("playing");
      const saved = (await api.progressAll()).find(([k]) => k === media.key);
      const pos = saved && saved[2] > 0 && saved[1] / saved[2] < 0.97 ? Math.max(0, saved[1] - 3) : 0;
      await api.mpv("set_property", "start", String(pos));
      await api.mpv("set_property", "pause", false);
      await api.mpv("set_property", "speed", 1);
      await api.mpv("loadfile", media.url);
    },
    [media],
  );

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const comps = await api.components();
        const has = (id: string) => comps.find((c) => c.id === id)?.installed ?? false;
        const listenOk = has("whisper") && has("whisper-model");
        setCanListen(listenOk);
        if (!listenOk) setAlign(false);
        api.llmStatus().then((s) => setLlm(s.available));
        await api.mpvStart();
        const existing = await api.prepared(media.key);
        if (cancelled) return;
        if (existing && existing.lang === lang) startPlayback(existing);
        else setPhase("choose");
      } catch (e) {
        setPrepError(String(e));
        setPhase("choose");
      }
    })();
    const un = listen<any>("prepare-progress", (e) => {
      if (e.payload.key === media.key) setPrep(e.payload);
    });
    return () => {
      cancelled = true;
      un.then((f) => f());
    };
  }, [media.key]);

  const runPrepare = async () => {
    setPhase("preparing");
    setPrepError("");
    setPrep({ stage: "extract", pct: 0, note: "Starting" });
    try {
      const p = await api.prepare(media, {
        target_lang: lang,
        native_lang: settings.nativeLang,
        target_sub: targetSub,
        native_sub: nativeSub,
        align,
      });
      if (p.note) toast(p.note);
      startPlayback(p);
    } catch (e) {
      setPrepError(String(e));
      setPhase("choose");
    }
  };

  // ---------- mpv events ----------
  useEffect(() => {
    const un = listen<any>("mpv", (e) => {
      const ev = e.payload;
      if (ev.event !== "property-change") return;
      switch (ev.name) {
        case "time-pos":
          if (typeof ev.data === "number") clock.current = { t: ev.data, stamp: performance.now() };
          break;
        case "pause":
          pausedRef.current = !!ev.data;
          clock.current = { t: now.current, stamp: performance.now() };
          if (!ev.data) holdIdx.current = -1;
          setPaused(!!ev.data);
          break;
        case "duration":
          if (typeof ev.data === "number") setDuration(ev.data);
          break;
        case "speed":
          if (typeof ev.data === "number") {
            clock.current = { t: now.current, stamp: performance.now() };
            speedRef.current = ev.data;
          }
          break;
        case "track-list":
          setAudioTracks((ev.data || []).filter((t: any) => t.type === "audio"));
          break;
        case "aid":
          setAid(typeof ev.data === "number" ? ev.data : null);
          break;
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Play the audio track the subtitles were matched against.
  useEffect(() => {
    if (!prepared || prepared.audio_index == null) return;
    const t = audioTracks.find((a) => a["ff-index"] === prepared.audio_index);
    if (t && t.id !== aid) api.mpv("set_property", "aid", t.id);
  }, [audioTracks, prepared]);

  // ---------- frame loop: current line, current word, auto-pause ----------
  useEffect(() => {
    if (phase !== "playing") return;
    let raf = 0;
    let lastUi = 0;
    const tick = () => {
      const c = clock.current;
      let t = c.t;
      if (!pausedRef.current) t += ((performance.now() - c.stamp) / 1000) * speedRef.current;
      now.current = t;
      const list = cuesRef.current;
      const s = live.current.settings;

      if (stopAt.current && t >= stopAt.current.t) {
        const then = stopAt.current.then;
        stopAt.current = null;
        api.mpv("set_property", "pause", true);
        then();
      }

      let idx = cueAt(list, t);
      if (pausedRef.current && holdIdx.current >= 0) idx = holdIdx.current;
      if (
        s.autoPause &&
        idx >= 0 &&
        !pausedRef.current &&
        !stopAt.current &&
        t >= list[idx].end - 0.1 &&
        autoPausedFor.current !== idx
      ) {
        autoPausedFor.current = idx;
        holdIdx.current = idx;
        pausedRef.current = true;
        api.mpv("set_property", "pause", true);
      }
      if (idx !== idxRef.current) {
        idxRef.current = idx;
        setCueIdx(idx);
        if (idx >= 0) {
          const w = live.current.words;
          const hard = list[idx].tokens.some((k) => k.w && k.lemma && (w.get(k.lemma) ?? 0) < 2);
          const want = live.current.baseSpeed * (s.slowUnknown && hard ? s.slowSpeed : 1);
          if (Math.abs(want - speedRef.current) > 0.01) api.mpv("set_property", "speed", want);
        }
      }
      const el = lineRef.current;
      if (el && idx >= 0) {
        for (const span of el.children as HTMLCollectionOf<HTMLElement>) {
          const ws = span.dataset.s;
          if (ws === undefined) continue;
          const start = +ws;
          const end = +(span.dataset.e as string);
          span.classList.toggle("now", t >= start && t < end);
          span.classList.toggle("said", t >= end);
        }
      }
      if (performance.now() - lastUi > 250) {
        lastUi = performance.now();
        setUiTime(t);
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [phase]);

  // Save position regularly and on exit.
  const saveProgress = useCallback(() => {
    if (now.current > 1) api.progressSet(media.key, media.title, now.current, live.current.duration);
  }, [media]);
  useEffect(() => {
    if (phase !== "playing") return;
    const id = window.setInterval(saveProgress, 10000);
    return () => window.clearInterval(id);
  }, [phase, saveProgress]);

  // Make room for the side panel.
  useEffect(() => {
    if (phase !== "playing") return;
    const apply = () =>
      api.mpv("set_property", "video-margin-ratio-right", panel === "none" ? 0 : Math.min(0.6, PANEL_W / window.innerWidth));
    apply();
    window.addEventListener("resize", apply);
    return () => window.removeEventListener("resize", apply);
  }, [panel, phase]);

  useEffect(() => {
    if (panel === "words") api.stats(media.key).then(setStats);
  }, [panel, media.key]);

  useEffect(() => {
    if (panel === "transcript") activeRow.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [cueIdx, panel]);

  // ---------- transport ----------
  const seek = (t: number) => {
    autoPausedFor.current = -1;
    holdIdx.current = -1;
    stopAt.current = null;
    clock.current = { t, stamp: performance.now() };
    api.mpv("seek", Math.max(0, t), "absolute+exact");
  };
  const playFrom = (t: number) => {
    seek(t);
    api.mpv("set_property", "pause", false);
  };
  const currentLine = () => (idxRef.current >= 0 ? idxRef.current : lastStarted(cuesRef.current, now.current));
  const replay = () => {
    const i = currentLine();
    if (i >= 0) playFrom(cuesRef.current[i].start);
  };
  const prevLine = () => {
    const i = currentLine();
    if (i < 0) return;
    // More than a moment into the line: go to its start first, like a music player.
    const target = now.current - cuesRef.current[i].start > 1.2 && idxRef.current < 0 ? i : Math.max(0, i - 1);
    playFrom(cuesRef.current[target].start);
  };
  const nextLine = () => {
    const i = lastStarted(cuesRef.current, now.current) + 1;
    if (i < cuesRef.current.length) playFrom(cuesRef.current[i].start);
  };
  const togglePause = () => api.mpv("set_property", "pause", !pausedRef.current);
  const changeSpeed = (v: number) => {
    setBaseSpeed(v);
    api.mpv("set_property", "speed", v);
  };
  const toggleFullscreen = async () => {
    const w = getCurrentWindow();
    const next = !(await w.isFullscreen());
    await w.setFullscreen(next);
    setFullscreen(next);
  };
  const exit = async () => {
    saveProgress();
    recorder.current?.stop();
    if (await getCurrentWindow().isFullscreen()) await getCurrentWindow().setFullscreen(false);
    await api.mpv("set_property", "video-margin-ratio-right", 0).catch(() => {});
    await api.mpv("stop").catch(() => {});
    onExit();
  };

  // ---------- word popup ----------
  const closePopup = () => {
    setPopup(null);
    if (popupPaused.current) {
      popupPaused.current = false;
      api.mpv("set_property", "pause", false);
    }
  };
  const openWord = (e: React.MouseEvent, ci: number, ti: number) => {
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    if (!pausedRef.current) {
      popupPaused.current = !hoverPaused.current;
      hoverPaused.current = false;
      api.mpv("set_property", "pause", true);
      holdIdx.current = ci;
    } else if (hoverPaused.current) {
      hoverPaused.current = false;
      popupPaused.current = true;
    }
    const toks = cuesRef.current[ci].tokens;
    if (e.shiftKey && popup && popup.cue === ci) {
      const from = Math.min(popup.from, ti);
      const to = Math.max(popup.to, ti);
      const text = toks.slice(from, to + 1).map((k) => k.t).join("");
      const next: Popup = { cue: ci, from, to, x: popup.x };
      setPopup(next);
      api.phrase(lang, text).then((phrase) => setPopup((p) => (p === next ? { ...next, phrase } : p)));
      return;
    }
    const next: Popup = { cue: ci, from: ti, to: ti, x: rect.left + rect.width / 2 };
    setPopup(next);
    api
      .lookup(lang, toks[ti].t)
      .then((lookup) => setPopup((p) => (p === next ? { ...next, lookup } : p)))
      .catch((err) => toast(String(err)));
  };
  const popupText = popup ? cues[popup.cue].tokens.slice(popup.from, popup.to + 1).map((k) => k.t).join("") : "";
  const popupLemma = popup && popup.from === popup.to ? popup.lookup?.lemma ?? cues[popup.cue].tokens[popup.from].lemma ?? null : null;

  const saveCard = (ci: number, word: string, lemma: string, definition: string) => {
    const cue = cuesRef.current[ci];
    if (!cue) return;
    api
      .cardAdd({
        lang,
        lemma,
        word,
        sentence: cue.text,
        translation: cue.en || translations[ci] || "",
        definition,
        media_key: media.key,
        media_title: media.title,
        start: cue.start,
        end: cue.end,
        url: media.url,
        audio_index: prepared?.audio_index ?? null,
      })
      .then(() => {
        toast(word ? `Saved “${word}” for review` : "Saved this line for review");
        if (lemma && (words.get(lemma) ?? 0) === 0) setWords([lemma], 1);
      })
      .catch((err) => toast(String(err)));
  };
  const explain = () => {
    if (!popup) return;
    const current = popup;
    setPopup({ ...current, explaining: true });
    api
      .explain(lang, popupText, cues[current.cue].text, "explain")
      .then((text) => setPopup((p) => (p && p.cue === current.cue && p.from === current.from ? { ...p, explain: text, explaining: false } : p)))
      .catch((err) => {
        toast(String(err));
        setPopup((p) => (p ? { ...p, explaining: false } : p));
      });
  };
  const translateLine = (ci: number) => {
    setTranslations((t) => ({ ...t, [ci]: "…" }));
    api
      .explain(lang, cuesRef.current[ci].text, "", "translate")
      .then((text) => setTranslations((t) => ({ ...t, [ci]: text })))
      .catch((err) => {
        toast(String(err));
        setTranslations((t) => {
          const n = { ...t };
          delete n[ci];
          return n;
        });
      });
  };

  // ---------- shadowing ----------
  const startShadow = () => {
    const ci = currentLine();
    if (ci < 0) return;
    const cue = cuesRef.current[ci];
    closePopup();
    setShadow({ phase: "listen", cue: ci });
    seek(cue.start);
    holdIdx.current = -1;
    api.mpv("set_property", "pause", false);
    stopAt.current = {
      t: cue.end,
      then: async () => {
        holdIdx.current = ci;
        try {
          const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
          const rec = new MediaRecorder(stream);
          const chunks: Blob[] = [];
          rec.ondataavailable = (ev) => chunks.push(ev.data);
          rec.onstop = async () => {
            stream.getTracks().forEach((tr) => tr.stop());
            recorder.current = null;
            const blob = new Blob(chunks, { type: rec.mimeType });
            const audioUrl = URL.createObjectURL(blob);
            setShadow({ phase: "scoring", cue: ci, audioUrl });
            try {
              const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
              const result = await api.shadow(bytes, cue.text, lang);
              setShadow({ phase: "result", cue: ci, audioUrl, result });
            } catch (err) {
              setShadow({ phase: "result", cue: ci, audioUrl, error: String(err) });
            }
          };
          recorder.current = rec;
          rec.start();
          setShadow({ phase: "record", cue: ci });
          window.setTimeout(() => rec.state === "recording" && rec.stop(), ((cue.end - cue.start) * 1.4 + 1.5) * 1000);
        } catch (err) {
          setShadow({ phase: "result", cue: ci, error: "Microphone not available: " + err });
        }
      },
    };
  };

  // ---------- keyboard and mouse ----------
  const keyHandler = useRef<(e: KeyboardEvent) => void>(() => {});
  keyHandler.current = (e: KeyboardEvent) => {
    if ((e.target as HTMLElement).tagName === "INPUT" || (e.target as HTMLElement).tagName === "SELECT") return;
    if (phase !== "playing") {
      if (e.key === "Escape") exit();
      return;
    }
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const actions: Record<string, () => void> = {
      " ": togglePause,
      ArrowLeft: prevLine,
      a: prevLine,
      ArrowRight: nextLine,
      d: nextLine,
      ArrowDown: replay,
      s: replay,
      ArrowUp: () => setReveal((r) => (r === currentLine() ? -1 : currentLine())),
      e: () => setReveal((r) => (r === currentLine() ? -1 : currentLine())),
      p: () => save({ autoPause: !settings.autoPause }),
      t: () => setPanel((x) => (x === "transcript" ? "none" : "transcript")),
      f: toggleFullscreen,
      w: () => {
        const ci = currentLine();
        if (ci >= 0) saveCard(ci, "", "", "");
      },
      r: startShadow,
      "[": () => changeSpeed(SPEEDS[Math.max(0, SPEEDS.indexOf(baseSpeed) - 1)] ?? 1),
      "]": () => changeSpeed(SPEEDS[Math.min(SPEEDS.length - 1, SPEEDS.indexOf(baseSpeed) + 1)] ?? 1),
      Escape: () => (popup ? closePopup() : shadow ? setShadow(null) : fullscreen ? toggleFullscreen() : exit()),
    };
    if (actions[k]) {
      e.preventDefault();
      actions[k]();
    }
  };
  useEffect(() => {
    const h = (e: KeyboardEvent) => keyHandler.current(e);
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  const wake = () => {
    setControls(true);
    window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => !pausedRef.current && setControls(false), 3000);
  };

  const onSubsEnter = () => {
    if (settings.pauseOnHover && !pausedRef.current && !shadow) {
      hoverPaused.current = true;
      holdIdx.current = idxRef.current;
      api.mpv("set_property", "pause", true);
    }
  };
  const onSubsLeave = () => {
    if (hoverPaused.current && !popup) {
      hoverPaused.current = false;
      api.mpv("set_property", "pause", false);
    }
  };

  // ---------- render helpers ----------
  const wordClass = (lemma?: string) => {
    if (!settings.colorWords || !lemma) return "w";
    const st = words.get(lemma) ?? 0;
    return "w " + (st === 0 ? "new" : st === 1 ? "learning" : "");
  };
  const cue = cueIdx >= 0 ? cues[cueIdx] : null;
  const english = cue ? cue.en || translations[cueIdx] || "" : "";
  const visibleCues = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const all = cues.map((c, i) => [c, i] as [Cue, number]);
    return q ? all.filter(([c]) => c.text.toLowerCase().includes(q) || c.en.toLowerCase().includes(q)) : all;
  }, [cues, filter]);
  const knownPct = stats && stats.words ? Math.round((stats.known / stats.words) * 100) : 0;

  if (phase !== "playing") {
    const targets = media.subs;
    const hasAuto = (l: string) => targets.some((t) => t.lang.slice(0, 2) === l && TEXT_CODECS.includes(t.codec.toLowerCase()));
    const langName = LANGS[lang] || lang;
    return (
      <div className="prep-screen">
        <div className="card prep">
          <h2>{media.title}</h2>
          {phase === "loading" && <p className="muted">Starting the player…</p>}
          {phase === "preparing" && (
            <>
              <p>{prep.note || "Working…"}</p>
              <div className="bar wide">
                <div style={{ width: prep.pct + "%" }} />
                <span>{Math.round(prep.pct)}%</span>
              </div>
              <p className="muted">
                {prep.stage === "listen"
                  ? "Speech recognition is running on your graphics card. This is done once per video."
                  : "Reading the video from your library. Large files over the network take a moment."}
              </p>
            </>
          )}
          {phase === "choose" && (
            <>
              {prepError && <div className="error">{prepError}</div>}
              <p className="muted">
                This video needs a one-time preparation before you can study with it.{" "}
                {hasAuto(lang) ? `${langName} subtitles were found.` : `No readable ${langName} subtitles were found.`}{" "}
                {hasAuto(settings.nativeLang) ? "English subtitles were found." : "No readable English subtitles were found."}
              </p>
              <label>
                {langName} subtitles
                <select value={targetSub} onChange={(e) => setTargetSub(e.target.value)}>
                  <option value="auto">{hasAuto(lang) ? "Automatic (best track)" : "Automatic"}</option>
                  {canListen && <option value="whisper">Write them from the audio (speech recognition)</option>}
                  {targets.map((t) => (
                    <option key={t.id} value={t.id} disabled={!TEXT_CODECS.includes(t.codec.toLowerCase())}>
                      {trackLabel(t)}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                English subtitles
                <select value={nativeSub} onChange={(e) => setNativeSub(e.target.value)}>
                  <option value="auto">Automatic</option>
                  <option value="none">None</option>
                  {targets.map((t) => (
                    <option key={t.id} value={t.id} disabled={!TEXT_CODECS.includes(t.codec.toLowerCase())}>
                      {trackLabel(t)}
                    </option>
                  ))}
                </select>
              </label>
              <label className="check">
                <input type="checkbox" checked={align} disabled={!canListen} onChange={(e) => setAlign(e.target.checked)} />
                <span>
                  Time every word from the audio
                  <span className="muted">
                    {canListen
                      ? " — highlights each word as it is spoken and corrects subtitle timing. Takes a few minutes."
                      : " — install speech recognition in Setup to enable this."}
                  </span>
                </span>
              </label>
              <div className="row end">
                <button className="ghost" onClick={exit}>
                  Back
                </button>
                <button className="primary" onClick={runPrepare}>
                  Prepare and play
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    );
  }

  const panelOpen = panel !== "none";
  return (
    <div className={"player" + (controls || paused ? " show-controls" : "")} onMouseMove={wake} onClick={() => popup && closePopup()}>
      <div className="stage" style={{ right: panelOpen ? PANEL_W : 0 }} onDoubleClick={toggleFullscreen}>
        <div className="click-catcher" onClick={() => (popup ? closePopup() : togglePause())} />

        {shadow && (
          <div className="shadow-box" onClick={(e) => e.stopPropagation()}>
            {shadow.phase === "listen" && <strong>Listen…</strong>}
            {shadow.phase === "record" && (
              <>
                <strong className="rec">● Your turn — say the line</strong>
                <button className="small" onClick={() => recorder.current?.stop()}>
                  Done
                </button>
              </>
            )}
            {shadow.phase === "scoring" && <strong>Checking your pronunciation…</strong>}
            {shadow.phase === "result" && (
              <>
                {shadow.result && (
                  <>
                    <div className="shadow-words">
                      {shadow.result.words.map(([w, ok], i) => (
                        <span key={i} className={ok ? "hit" : "miss"}>
                          {w}{" "}
                        </span>
                      ))}
                    </div>
                    <div className="muted">
                      {Math.round(shadow.result.score * 100)}% of words recognised · heard: “{shadow.result.heard || "nothing"}”
                    </div>
                  </>
                )}
                {shadow.error && <div className="muted">{shadow.error}</div>}
                <div className="row">
                  {shadow.audioUrl && (
                    <button className="small" onClick={() => new Audio(shadow.audioUrl).play()}>
                      Play mine
                    </button>
                  )}
                  <button className="small" onClick={() => playFrom(cues[shadow.cue].start)}>
                    Hear original
                  </button>
                  <button className="small" onClick={startShadow}>
                    Try again
                  </button>
                  <button className="ghost small" onClick={() => setShadow(null)}>
                    Close
                  </button>
                </div>
              </>
            )}
          </div>
        )}

        <div className="subs" onMouseEnter={onSubsEnter} onMouseLeave={onSubsLeave} onClick={(e) => e.stopPropagation()}>
          {cue && (
            <>
              <div className="line target" ref={lineRef} style={{ fontSize: settings.subSize }}>
                {cue.tokens.map((tok, i) =>
                  tok.w ? (
                    <span
                      key={cueIdx + "-" + i}
                      className={
                        wordClass(tok.lemma) + (popup && popup.cue === cueIdx && i >= popup.from && i <= popup.to ? " picked" : "")
                      }
                      data-s={tok.s}
                      data-e={tok.e}
                      onClick={(e) => openWord(e, cueIdx, i)}
                    >
                      {tok.t}
                    </span>
                  ) : (
                    <span key={cueIdx + "-" + i}>{tok.t}</span>
                  ),
                )}
              </div>
              {settings.english !== "hide" &&
                (english ? (
                  <div
                    className={"line native" + (settings.english === "blur" && reveal !== cueIdx ? " blur" : "")}
                    style={{ fontSize: settings.subSize * 0.62 }}
                    onClick={() => setReveal(cueIdx)}
                  >
                    {english}
                  </div>
                ) : (
                  llm && (
                    <button className="ghost small translate" onClick={() => translateLine(cueIdx)}>
                      Translate this line
                    </button>
                  )
                ))}
            </>
          )}
        </div>

        {popup && cues[popup.cue] && (
          <div
            className="popup"
            style={{ left: Math.max(190, Math.min(popup.x, window.innerWidth - (panelOpen ? PANEL_W : 0) - 190)) }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="popup-head">
              <span className="popup-word">{popupText}</span>
              {popupLemma && popupLemma !== popupText.toLowerCase() && <span className="muted">→ {popupLemma}</span>}
              {popup.lookup?.entries[0]?.ipa && <span className="ipa">{popup.lookup.entries[0].ipa}</span>}
              <div className="spacer" />
              <button className="ghost small" onClick={closePopup}>
                ✕
              </button>
            </div>
            {popup.lookup &&
              (() => {
                const f = freqLabel(popup.lookup.rank);
                return f ? <div className={"freq f" + f.level}>{"●".repeat(f.level + 1)} {f.label}</div> : null;
              })()}
            <div className="popup-body">
              {(popup.from === popup.to ? popup.lookup?.entries : popup.phrase)?.slice(0, 4).map((en, i) => (
                <div key={i} className="entry">
                  <div className="entry-head">
                    <strong>{en.word}</strong> <em>{en.pos}</em>
                    {en.gender && <span className={"gender " + (en.gender === "f" ? "fem" : en.gender === "m" ? "masc" : "")}>{en.gender === "f" ? "feminine" : en.gender === "m" ? "masculine" : en.gender}</span>}
                  </div>
                  {en.form_tags.length > 0 && <div className="form-tags">{popupText.toLowerCase()} = {en.form_tags.join("; ")}</div>}
                  <ol>
                    {en.glosses.slice(0, 4).map((g, j) => (
                      <li key={j}>{g}</li>
                    ))}
                  </ol>
                </div>
              ))}
              {popup.from === popup.to && popup.lookup && popup.lookup.entries.length === 0 && (
                <div className="muted">Not in the dictionary (probably a name).</div>
              )}
              {popup.from !== popup.to && popup.phrase && popup.phrase.length === 0 && (
                <div className="muted">No dictionary entry for this exact phrase{llm ? " — try Explain." : "."}</div>
              )}
              {popup.explaining && <div className="muted">Thinking…</div>}
              {popup.explain && <div className="explain">{popup.explain}</div>}
            </div>
            {popupLemma && (
              <div className="status-row">
                {STATUS.map((label, st) => (
                  <button
                    key={st}
                    className={"small " + ((words.get(popupLemma) ?? 0) === st ? "primary" : "ghost")}
                    onClick={() => setWords([popupLemma], st)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
            <div className="row">
              <button
                className="small"
                onClick={() => {
                  const entries = popup.from === popup.to ? popup.lookup?.entries : popup.phrase;
                  const def = (entries?.[0]?.glosses || []).slice(0, 3).join("; ");
                  saveCard(popup.cue, popupText, popupLemma || "", def || popup.explain || "");
                }}
              >
                ★ Save for review
              </button>
              <button className="small" disabled={!llm || popup.explaining} title={llm ? "" : "Install the explanation model in Setup"} onClick={explain}>
                Explain in context
              </button>
              <span className="muted small-text">Shift-click another word to select a phrase</span>
            </div>
          </div>
        )}

        <div className="controls" onClick={(e) => e.stopPropagation()}>
          <input
            className="seek"
            type="range"
            min={0}
            max={duration || 1}
            step={0.5}
            value={Math.min(uiTime, duration || 1)}
            onChange={(e) => seek(+e.target.value)}
          />
          <div className="row">
            <button className="ghost" onClick={exit} title="Back to library (Esc)">←</button>
            <button onClick={togglePause} title="Play / pause (Space)">{paused ? "▶" : "❚❚"}</button>
            <button className="ghost" onClick={prevLine} title="Previous line (A or ←)">⏮</button>
            <button className="ghost" onClick={replay} title="Replay this line (S or ↓)">↺</button>
            <button className="ghost" onClick={nextLine} title="Next line (D or →)">⏭</button>
            <span className="time">{fmtTime(uiTime)} / {fmtTime(duration)}</span>
            <span className="title">{media.title}{prepared && !prepared.aligned ? " · word timing estimated" : ""}</span>
            <div className="spacer" />
            <select value={baseSpeed} onChange={(e) => changeSpeed(+e.target.value)} title="Speed ([ and ])">
              {SPEEDS.map((v) => (
                <option key={v} value={v}>{v}×</option>
              ))}
            </select>
            {audioTracks.length > 1 && (
              <select value={aid ?? ""} onChange={(e) => api.mpv("set_property", "aid", +e.target.value)} title="Audio track">
                {audioTracks.map((t) => (
                  <option key={t.id} value={t.id}>{(t.lang || "audio") + (t.title ? " · " + t.title : "")}</option>
                ))}
              </select>
            )}
            <button className={settings.autoPause ? "toggle on" : "toggle"} onClick={() => save({ autoPause: !settings.autoPause })} title="Pause after every line (P)">Auto-pause</button>
            <button className={settings.slowUnknown ? "toggle on" : "toggle"} onClick={() => save({ slowUnknown: !settings.slowUnknown })} title="Slow down lines with words you do not know yet">Slow hard lines</button>
            <button
              className="toggle on"
              onClick={() => save({ english: settings.english === "show" ? "blur" : settings.english === "blur" ? "hide" : "show" })}
              title="English line: shown, blurred until you click it or press E, or hidden"
            >
              EN: {settings.english}
            </button>
            <button className="ghost" onClick={startShadow} title="Shadowing: hear the line, then record yourself (R)">🎤</button>
            <button className="ghost" onClick={() => { const ci = currentLine(); if (ci >= 0) saveCard(ci, "", "", ""); }} title="Save this line for review (W)">★</button>
            <button className={panelOpen ? "toggle on" : "toggle"} onClick={() => setPanel(panelOpen ? "none" : "transcript")} title="Transcript and word list (T)">Transcript</button>
            <button className="ghost" onClick={toggleFullscreen} title="Fullscreen (F)">⛶</button>
          </div>
        </div>
      </div>

      {panelOpen && (
        <aside className="panel" style={{ width: PANEL_W }} onClick={(e) => e.stopPropagation()}>
          <div className="panel-tabs">
            <button className={panel === "transcript" ? "tab active" : "tab"} onClick={() => setPanel("transcript")}>Transcript</button>
            <button className={panel === "words" ? "tab active" : "tab"} onClick={() => setPanel("words")}>Words to study</button>
            <div className="spacer" />
            <button className="ghost small" onClick={() => setPanel("none")}>✕</button>
          </div>
          {panel === "transcript" && (
            <>
              <input placeholder="Search the transcript…" value={filter} onChange={(e) => setFilter(e.target.value)} />
              <div className="rows">
                {visibleCues.map(([c, i]) => (
                  <div key={i} ref={i === cueIdx ? activeRow : undefined} className={i === cueIdx ? "trow active" : "trow"} onClick={() => playFrom(c.start)}>
                    <span className="ttime">{fmtTime(c.start)}</span>
                    <div>
                      <div>{c.text}</div>
                      {settings.english !== "hide" && c.en && <div className={"ten" + (settings.english === "blur" ? " blur" : "")}>{c.en}</div>}
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
          {panel === "words" && (
            <div className="rows">
              {!stats && <div className="muted pad">Counting words…</div>}
              {stats && (
                <>
                  <div className="pad">
                    <strong>You know {knownPct}% of the words spoken here.</strong>
                    <div className="muted">
                      {stats.unique_known} of {stats.unique} different words · {stats.lines} lines. These are the ones you have not marked as known, most frequent in this video first.
                    </div>
                  </div>
                  {stats.unknown.map((u) => (
                    <div key={u.lemma} className="wrow" title={u.example}>
                      <div className="wmain">
                        <strong>{u.lemma}</strong> <span className="muted">×{u.count}</span>
                        <div className="muted">{u.gloss}</div>
                      </div>
                      <button
                        className="ghost small"
                        onClick={() => {
                          setWords([u.lemma], 2);
                          setStats({ ...stats, known: stats.known + u.count, unique_known: stats.unique_known + 1, unknown: stats.unknown.filter((x) => x !== u) });
                        }}
                      >
                        I know this
                      </button>
                      <button
                        className="ghost small"
                        onClick={() => {
                          setWords([u.lemma], 3);
                          setStats({ ...stats, unknown: stats.unknown.filter((x) => x !== u) });
                        }}
                        title="Names and noise"
                      >
                        Ignore
                      </button>
                    </div>
                  ))}
                </>
              )}
              <div className="pad">
                <button
                  className="ghost small"
                  onClick={async () => {
                    await api.mpv("stop");
                    await api.preparedDelete(media.key);
                    setPrepared(null);
                    setPanel("none");
                    setPhase("choose");
                  }}
                >
                  Prepare this video again with different subtitles
                </button>
              </div>
            </div>
          )}
        </aside>
      )}
    </div>
  );
}
