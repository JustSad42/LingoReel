import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { check, Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api, DEFAULTS, MediaReq, Settings } from "./api";
import Setup from "./views/Setup";
import Library from "./views/Library";
import Player from "./views/Player";
import Review from "./views/Review";
import Words from "./views/Words";
import SettingsView from "./views/SettingsView";
import QueueView from "./views/Queue";
import { useQueue } from "./queue";
import Icon from "./Icon";
import Backdrop from "./Backdrop";
import { getCurrentWebview } from "@tauri-apps/api/webview";

/** Zoom currently applied to the webview. */
let zoom = 1;

type Ctx = {
  settings: Settings;
  save: (patch: Partial<Settings>) => void;
  /** lemma -> status for the target language */
  words: Map<string, number>;
  setWords: (lemmas: string[], status: number) => void;
  toast: (message: string) => void;
  play: (media: MediaReq) => void;
};

const AppCtx = createContext<Ctx>(null as unknown as Ctx);
export const useApp = () => useContext(AppCtx);

type View = "library" | "queue" | "review" | "words" | "setup" | "settings";

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [words, setWordMap] = useState<Map<string, number>>(new Map());
  const [view, setView] = useState<View>("library");
  const [playing, setPlaying] = useState<MediaReq | null>(null);
  const [toastMsg, setToastMsg] = useState("");
  const [update, setUpdate] = useState<Update | null>(null);
  const [updating, setUpdating] = useState(false);
  const [dueCount, setDueCount] = useState(0);
  const queue = useQueue();
  const pending = queue.items.filter((i) => i.state !== "failed").length;
  const busy = queue.items.some((i) => i.state === "working");
  const toastTimer = useRef<number>();

  useEffect(() => {
    (async () => {
      const stored = await api.settingsGet();
      const merged: Settings = { ...DEFAULTS, ...stored };
      if (!merged.clientId) merged.clientId = crypto.randomUUID();
      setSettings(merged);
      await api.settingsSet(merged);
      const comps = await api.components();
      if (comps.some((c) => c.required && !c.installed)) setView("setup");
    })();
    check().then(setUpdate).catch(() => {});
  }, []);

  const lang = settings?.targetLang;
  const firstLang = useRef(true);
  useEffect(() => {
    if (!lang) return;
    api.wordsAll(lang).then((rows) => setWordMap(new Map(rows)));
    if (firstLang.current) {
      firstLang.current = false;
      return;
    }
    // Switched language: its dictionary may still need downloading.
    api.components().then((comps) => {
      if (comps.some((c) => c.required && !c.installed)) {
        setView("setup");
        toast("Download the dictionary for this language to start studying it.");
      }
    });
  }, [lang]);

  useEffect(() => {
    if (!lang || playing) return;
    api.cardsCounts(lang).then((c) => setDueCount(c.due + c.new)).catch(() => {});
  }, [lang, view, playing]);

  const save = useCallback((patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...(prev as Settings), ...patch };
      api.settingsSet(next);
      return next;
    });
  }, []);

  const setWords = useCallback(
    (lemmas: string[], status: number) => {
      if (!lang || lemmas.length === 0) return;
      setWordMap((prev) => {
        const next = new Map(prev);
        for (const l of lemmas) status === 0 ? next.delete(l) : next.set(l, status);
        return next;
      });
      api.wordsSet(lang, lemmas, status);
    },
    [lang],
  );

  const toast = useCallback((message: string) => {
    setToastMsg(message);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToastMsg(""), 3500);
  }, []);

  const ctx = useMemo<Ctx | null>(
    () => (settings ? { settings, save, words, setWords, toast, play: setPlaying } : null),
    [settings, save, words, setWords, toast],
  );

  useEffect(() => {
    document.documentElement.classList.toggle("player-active", !!playing);
  }, [playing]);

  // Scale the whole interface with the window, so it is not tiny on a large or 4K screen.
  const uiScale = settings?.uiScale;
  useEffect(() => {
    if (!uiScale) return;
    let timer = 0;
    const apply = () => {
      // Window size in unzoomed units stays the same whatever zoom is applied.
      const w = window.innerWidth * zoom;
      const h = window.innerHeight * zoom;
      const auto = Math.min(w / 1440, h / 840);
      const want = uiScale === "auto" ? Math.round(Math.min(2.4, Math.max(0.8, auto)) * 20) / 20 : Number(uiScale) || 1;
      if (Math.abs(want - zoom) > 0.01) {
        zoom = want;
        getCurrentWebview().setZoom(want).catch(() => {});
      }
    };
    const onResize = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(apply, 120);
    };
    apply();
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      window.clearTimeout(timer);
    };
  }, [uiScale]);

  if (!ctx) return null;

  const installUpdate = async () => {
    if (!update) return;
    setUpdating(true);
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (e) {
      toast("Update failed: " + e);
      setUpdating(false);
    }
  };

  const tabs: [View, string, string][] = [
    ["library", "Library", "library"],
    ["queue", "Queue", "queue"],
    ["review", "Review", "review"],
    ["words", "Words", "words"],
    ["setup", "Setup", "download"],
    ["settings", "Settings", "settings"],
  ];

  return (
    <AppCtx.Provider value={ctx}>
      {playing && <Player key={playing.key} media={playing} onExit={() => setPlaying(null)} />}
      {
        <div className="shell" style={{ display: playing ? "none" : undefined }}>
          <Backdrop />
          <nav className="rail">
            <img className="logo" src="/logo.png" alt="LingoReel" />
            {tabs.map(([id, label, icon]) => (
              <button key={id} className={view === id ? "rail-item active" : "rail-item"} onClick={() => setView(id)}>
                <Icon name={icon} size={21} />
                <span>{label}</span>
                {id === "review" && dueCount > 0 && <em className="badge">{dueCount}</em>}
                {id === "queue" && pending > 0 && (
                  <em className={"badge" + (busy ? " busy" : "")} title={busy ? "Preparing in the background" : undefined}>
                    {pending}
                  </em>
                )}
              </button>
            ))}
            <div className="spacer" />
            {update && (
              <button className="rail-item update" disabled={updating} onClick={installUpdate} title={`Version ${update.version} is available`}>
                <Icon name="download" size={21} />
                <span>{updating ? "Updating" : "Update"}</span>
              </button>
            )}
          </nav>
          <main className="main">
            {/* The library stays loaded so returning to it is instant. */}
            <div className="view" hidden={view !== "library"}>
              <Library active={view === "library" && !playing} />
            </div>
            {view !== "library" && !playing && (
              <div className="view enter" key={view}>
                {view === "queue" && <QueueView />}
                {view === "review" && <Review />}
                {view === "words" && <Words />}
                {view === "setup" && <Setup onDone={() => setView("library")} />}
                {view === "settings" && <SettingsView />}
              </div>
            )}
          </main>
        </div>
      }
      {toastMsg && (
        <div className="toast" key={toastMsg}>
          {toastMsg}
        </div>
      )}
    </AppCtx.Provider>
  );
}
