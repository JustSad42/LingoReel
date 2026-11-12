import { useEffect, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../api";
import { useApp } from "../App";

export default function SettingsView() {
  const { settings, save, toast } = useApp();
  const [info, setInfo] = useState({ version: "", dataDir: "", nvidia: false });
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    api.info().then(setInfo);
  }, []);

  const checkUpdate = async () => {
    setChecking(true);
    try {
      const u = await check();
      if (!u) toast("You have the latest version.");
      else {
        toast(`Downloading version ${u.version}…`);
        await u.downloadAndInstall();
        await relaunch();
      }
    } catch (e) {
      toast("Could not check for updates: " + e);
    }
    setChecking(false);
  };

  const toggle = (key: "autoPause" | "pauseOnHover" | "slowUnknown" | "align" | "colorWords", label: string, hint: string) => (
    <label className="check">
      <input type="checkbox" checked={settings[key]} onChange={(e) => save({ [key]: e.target.checked })} />
      <span>
        {label}
        <span className="muted"> — {hint}</span>
      </span>
    </label>
  );

  return (
    <div className="page narrow">
      <h1>Settings</h1>

      <section className="card">
        <h2>Watching</h2>
        {toggle("pauseOnHover", "Pause when the mouse is over the subtitles", "so you can read and click words without rushing")}
        {toggle("autoPause", "Pause after every line", "press Space to continue; useful for careful study")}
        {toggle("slowUnknown", "Slow down lines with words you do not know", "plays those lines at the speed below")}
        <label>
          Slow speed: {settings.slowSpeed}×
          <input type="range" min={0.5} max={0.95} step={0.05} value={settings.slowSpeed} onChange={(e) => save({ slowSpeed: +e.target.value })} />
        </label>
        {toggle("colorWords", "Underline words by how well you know them", "orange is new, yellow is learning, plain is known")}
        <label>
          English line
          <select value={settings.english} onChange={(e) => save({ english: e.target.value })}>
            <option value="show">Always shown</option>
            <option value="blur">Blurred until you click it or press E</option>
            <option value="hide">Hidden</option>
          </select>
        </label>
        <label>
          Subtitle size: {settings.subSize}px
          <input type="range" min={20} max={60} step={1} value={settings.subSize} onChange={(e) => save({ subSize: +e.target.value })} />
        </label>
        {toggle("align", "Time every word from the audio by default", "applies when preparing a video; needs speech recognition from Setup")}
      </section>

      <section className="card">
        <h2>Languages</h2>
        <label>
          I am learning
          <select value={settings.targetLang} onChange={(e) => save({ targetLang: e.target.value })}>
            <option value="fr">French</option>
          </select>
        </label>
        <p className="muted">
          French is fully supported today. Japanese needs its own word splitter and dictionary and is planned as a later
          update.
        </p>
      </section>

      <section className="card">
        <h2>Plex</h2>
        {settings.plex ? (
          <div className="row">
            <span>
              Connected to <strong>{settings.plex.name}</strong> <span className="muted">({settings.plex.url})</span>
            </span>
            <div className="spacer" />
            <button className="ghost small" onClick={() => save({ plex: null })}>
              Disconnect
            </button>
          </div>
        ) : (
          <p className="muted">Not connected. Use “Connect Plex” in the Library.</p>
        )}
        <p className="muted">
          Optional: videos play straight from Plex over your network. If this PC can also open the files directly (a
          network share), map the server's folder to the share and LingoReel will read the files itself, which prepares
          videos faster.
        </p>
        <label>
          Folder as Plex sees it
          <input placeholder="/data/media" value={settings.pathFrom} onChange={(e) => save({ pathFrom: e.target.value })} />
        </label>
        <label>
          Same folder from this PC
          <input placeholder="\\TOWER\media" value={settings.pathTo} onChange={(e) => save({ pathTo: e.target.value })} />
        </label>
      </section>

      <section className="card">
        <h2>Explanations</h2>
        <p className="muted">
          By default LingoReel runs its own model (download it in Setup). If you already run Ollama or LM Studio, enter
          its local address instead. Explanations never use a cloud service.
        </p>
        <label>
          Local endpoint (optional)
          <input placeholder="http://localhost:11434/v1" value={settings.llmUrl} onChange={(e) => save({ llmUrl: e.target.value })} />
        </label>
        <label>
          Model name for that endpoint
          <input placeholder="gemma3:4b" value={settings.llmModel} onChange={(e) => save({ llmModel: e.target.value })} />
        </label>
      </section>

      <section className="card">
        <h2>About</h2>
        <div className="row">
          <span>LingoReel {info.version}</span>
          <div className="spacer" />
          <button className="small" disabled={checking} onClick={checkUpdate}>
            {checking ? "Checking…" : "Check for updates"}
          </button>
        </div>
        <p className="muted">Your data is stored in {info.dataDir}</p>
        <p className="muted">
          Dictionary data from Wiktionary via kaikki.org and word frequencies from FrequencyWords (both CC BY-SA). Uses mpv,
          FFmpeg, whisper.cpp and llama.cpp.
        </p>
      </section>
    </div>
  );
}
