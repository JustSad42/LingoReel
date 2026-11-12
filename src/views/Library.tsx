import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, FileEntry, fmtTime, MediaStats, plex, PlexConn } from "../api";
import { useApp } from "../App";

type Source = { kind: "plex"; key: string; title: string; type: string } | { kind: "folder"; path: string };
type Item = any;

function DifficultyBadge({ s }: { s?: MediaStats }) {
  if (!s) return null;
  const pct = s.words ? Math.round((s.known / s.words) * 100) : 0;
  const level = pct >= 90 ? "easy" : pct >= 75 ? "medium" : "hard";
  return (
    <span className={"diff " + level} title={`You know ${pct}% of the words spoken (${s.unique_known} of ${s.unique} different words)`}>
      {pct}% known{s.aligned ? " · word-timed" : ""}
    </span>
  );
}

export default function Library() {
  const { settings, save, toast, play } = useApp();
  const p = settings.plex;
  const [sections, setSections] = useState<Item[]>([]);
  const [source, setSource] = useState<Source | null>(null);
  const [items, setItems] = useState<Item[]>([]);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [show, setShow] = useState<Item | null>(null);
  const [episodes, setEpisodes] = useState<Item[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [summaries, setSummaries] = useState<Record<string, MediaStats>>({});
  const [progress, setProgress] = useState<Map<string, [number, number]>>(new Map());
  const [linking, setLinking] = useState(false);
  const [batch, setBatch] = useState<{ key: string; label: string; pct: number } | null>(null);
  const cancelBatch = useRef(false);

  useEffect(() => {
    api.progressAll().then((rows) => setProgress(new Map(rows.map(([k, pos, dur]) => [k, [pos, dur]]))));
  }, []);

  useEffect(() => {
    if (!p) {
      setSections([]);
      return;
    }
    plex
      .get(p, "/library/sections")
      .then((mc) => {
        const dirs = (mc.Directory || []).filter((d: Item) => d.type === "movie" || d.type === "show");
        setSections(dirs);
        setError("");
        setSource((cur) => cur ?? (dirs[0] ? { kind: "plex", key: dirs[0].key, title: dirs[0].title, type: dirs[0].type } : null));
      })
      .catch((e) => setError(`Could not reach your Plex server at ${p.url}. ${e}`));
  }, [p?.url, p?.token]);

  useEffect(() => {
    if (!source) return;
    setShow(null);
    setQuery("");
    setLoading(true);
    const done = () => setLoading(false);
    if (source.kind === "plex" && p) {
      plex
        .get(p, `/library/sections/${source.key}/all`)
        .then((mc) => setItems(mc.Metadata || []))
        .catch((e) => setError(String(e)))
        .finally(done);
    } else if (source.kind === "folder") {
      api
        .scanFolder(source.path)
        .then(setFiles)
        .catch((e) => setError(String(e)))
        .finally(done);
    }
  }, [source]);

  useEffect(() => {
    if (!show || !p) return;
    setLoading(true);
    plex
      .get(p, `/library/metadata/${show.ratingKey}/allLeaves`)
      .then((mc) => setEpisodes(mc.Metadata || []))
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [show]);

  // Difficulty badges for whatever is on screen.
  const visibleKeys = useMemo(() => {
    if (show) return episodes.map((e) => `plex-${e.ratingKey}`);
    if (source?.kind === "folder") return files.map((f) => f.key);
    if (source?.kind === "plex" && source.type === "movie") return items.map((i) => `plex-${i.ratingKey}`);
    return [];
  }, [show, episodes, files, items, source]);
  const refreshSummaries = () => {
    if (visibleKeys.length) api.summaries(visibleKeys).then(setSummaries).catch(() => {});
  };
  useEffect(refreshSummaries, [visibleKeys]);

  useEffect(() => {
    const un = listen<{ key: string; stage: string; pct: number }>("prepare-progress", (e) => {
      const stageShare: Record<string, [number, number]> = { extract: [0, 25], listen: [25, 70], align: [95, 5], done: [100, 0] };
      const [base, span] = stageShare[e.payload.stage] || [0, 0];
      setBatch((b) => (b && b.key === e.payload.key ? { ...b, pct: base + (span * e.payload.pct) / 100 } : b));
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const connectPlex = async () => {
    setLinking(true);
    try {
      const pin = await plex.pinCreate(settings);
      await openUrl(pin.url);
      let token: string | null = null;
      for (let i = 0; i < 90 && !token; i++) {
        await new Promise((r) => setTimeout(r, 2000));
        token = await plex.pinToken(settings, pin.id).catch(() => null);
      }
      if (!token) throw new Error("Sign-in was not completed in time.");
      const servers = await plex.servers(settings, token);
      if (servers.length === 0) throw new Error("Signed in, but no Plex server could be reached from this PC.");
      save({ plex: servers[0] });
      toast(`Connected to ${servers[0].name}`);
    } catch (e) {
      toast(String(e));
    }
    setLinking(false);
  };

  const addFolder = async () => {
    const dir = await open({ directory: true, title: "Choose a folder of videos" });
    if (typeof dir === "string") {
      if (!settings.folders.includes(dir)) save({ folders: [...settings.folders, dir] });
      setSource({ kind: "folder", path: dir });
    }
  };

  const openFile = async () => {
    const file = await open({
      title: "Open a video",
      filters: [{ name: "Video", extensions: ["mkv", "mp4", "m4v", "avi", "mov", "webm", "ts"] }],
    });
    if (typeof file === "string") playFile(file);
  };

  const playFile = (path: string) =>
    api
      .probeFile(path)
      .then(play)
      .catch((e) => toast(String(e)));

  const playPlex = (ratingKey: string) =>
    plex
      .media(p as PlexConn, settings, ratingKey)
      .then(play)
      .catch((e) => toast(String(e)));

  /** Prepare every listed episode in the background so they are ready (and scored) before watching. */
  const prepareAll = async (list: Item[]) => {
    cancelBatch.current = false;
    for (const ep of list) {
      if (cancelBatch.current) break;
      const key = `plex-${ep.ratingKey}`;
      if (summaries[key]) continue;
      setBatch({ key, label: `S${ep.parentIndex}E${ep.index} ${ep.title}`, pct: 0 });
      try {
        const req = await plex.media(p as PlexConn, settings, ep.ratingKey);
        await api.prepare(req, {
          target_lang: settings.targetLang,
          native_lang: settings.nativeLang,
          target_sub: "auto",
          native_sub: "auto",
          align: settings.align,
        });
        const s = await api.summaries([key]);
        setSummaries((prev) => ({ ...prev, ...s }));
      } catch (e) {
        toast(`${ep.title}: ${e}`);
      }
    }
    setBatch(null);
  };

  const q = query.trim().toLowerCase();
  const filtered = q ? items.filter((i) => String(i.title).toLowerCase().includes(q)) : items;
  const seasons = useMemo(() => {
    const m = new Map<number, Item[]>();
    for (const e of episodes) {
      const s = e.parentIndex ?? 0;
      m.set(s, [...(m.get(s) || []), e]);
    }
    return [...m.entries()].sort((a, b) => a[0] - b[0]);
  }, [episodes]);

  const resume = (key: string, fallbackOffsetMs?: number, durMs?: number) => {
    const local = progress.get(key);
    const pos = local ? local[0] : (fallbackOffsetMs || 0) / 1000;
    const dur = local ? local[1] : (durMs || 0) / 1000;
    if (!pos || !dur || pos / dur < 0.02) return null;
    return <div className="resume"><div style={{ width: Math.min(100, (pos / dur) * 100) + "%" }} /></div>;
  };

  return (
    <div className="library">
      <aside className="side">
        <h3>Plex</h3>
        {p ? (
          <>
            {sections.map((s) => (
              <button
                key={s.key}
                className={source?.kind === "plex" && source.key === s.key ? "side-item active" : "side-item"}
                onClick={() => setSource({ kind: "plex", key: s.key, title: s.title, type: s.type })}
              >
                {s.title}
              </button>
            ))}
            <div className="muted small-text">{p.name}</div>
          </>
        ) : (
          <button className="primary small" disabled={linking} onClick={connectPlex}>
            {linking ? "Waiting for sign-in in your browser…" : "Connect Plex"}
          </button>
        )}
        <h3>Folders</h3>
        {settings.folders.map((f) => (
          <button
            key={f}
            className={source?.kind === "folder" && source.path === f ? "side-item active" : "side-item"}
            onClick={() => setSource({ kind: "folder", path: f })}
            title={f}
          >
            {f.split(/[\\/]/).filter(Boolean).pop()}
          </button>
        ))}
        <button className="ghost small" onClick={addFolder}>
          Add folder…
        </button>
        <button className="ghost small" onClick={openFile}>
          Open a video file…
        </button>
      </aside>

      <section className="content">
        {error && <div className="error">{error}</div>}
        {!source && !error && (
          <div className="empty">
            <h2>Add your library</h2>
            <p className="muted">Connect your Plex server, or add a folder of videos, to start watching.</p>
          </div>
        )}

        {source && !show && (
          <div className="row head">
            <h1>{source.kind === "plex" ? source.title : source.path}</h1>
            <div className="spacer" />
            {source.kind === "plex" && (
              <input placeholder="Search…" value={query} onChange={(e) => setQuery(e.target.value)} />
            )}
            {source.kind === "folder" && (
              <button
                className="ghost small"
                onClick={() => {
                  save({ folders: settings.folders.filter((f) => f !== source.path) });
                  setSource(null);
                }}
              >
                Remove folder
              </button>
            )}
          </div>
        )}
        {loading && <div className="muted">Loading…</div>}

        {source?.kind === "plex" && !show && p && (
          <div className="grid">
            {filtered.map((i) => {
              const key = `plex-${i.ratingKey}`;
              return (
                <button
                  key={i.ratingKey}
                  className="poster"
                  onClick={() => (i.type === "show" ? setShow(i) : playPlex(i.ratingKey))}
                >
                  <div className="poster-img">
                    {i.thumb && <img loading="lazy" src={plex.thumb(p, i.thumb)} alt="" />}
                    {i.type !== "show" && resume(key, i.viewOffset, i.duration)}
                  </div>
                  <div className="poster-title">{i.title}</div>
                  <div className="muted small-text">
                    {i.year}
                    {i.type === "show" && i.leafCount ? ` · ${i.leafCount} episodes` : ""}
                  </div>
                  <DifficultyBadge s={summaries[key]} />
                </button>
              );
            })}
          </div>
        )}

        {show && p && (
          <div>
            <div className="row head">
              <button className="ghost small" onClick={() => setShow(null)}>
                ← Back
              </button>
              <h1>{show.title}</h1>
              <div className="spacer" />
              {batch ? (
                <>
                  <span className="muted small-text">Preparing {batch.label}</span>
                  <div className="bar">
                    <div style={{ width: batch.pct + "%" }} />
                    <span>{Math.round(batch.pct)}%</span>
                  </div>
                  <button className="ghost small" onClick={() => (cancelBatch.current = true)}>
                    Stop after this one
                  </button>
                </>
              ) : (
                <span className="muted small-text">Prepare episodes ahead of time to see how hard each one is.</span>
              )}
            </div>
            {seasons.map(([num, eps]) => (
              <div key={num} className="season">
                <div className="row">
                  <h2>{num === 0 ? "Specials" : `Season ${num}`}</h2>
                  <div className="spacer" />
                  <button className="ghost small" disabled={!!batch} onClick={() => prepareAll(eps)}>
                    Prepare season
                  </button>
                </div>
                {eps.map((e) => {
                  const key = `plex-${e.ratingKey}`;
                  return (
                    <button key={e.ratingKey} className="episode" onClick={() => playPlex(e.ratingKey)}>
                      <span className="ep-num">{e.index}</span>
                      <span className="ep-title">
                        {e.title}
                        {resume(key, e.viewOffset, e.duration)}
                      </span>
                      <DifficultyBadge s={summaries[key]} />
                      <span className="muted">{fmtTime((e.duration || 0) / 1000)}</span>
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        )}

        {source?.kind === "folder" && (
          <div>
            {files.length === 0 && !loading && <div className="muted">No video files found in this folder.</div>}
            {files.map((f, i) => (
              <div key={f.path}>
                {(i === 0 || files[i - 1].folder !== f.folder) && f.folder && <h2 className="folder-head">{f.folder}</h2>}
                <button className="episode" onClick={() => playFile(f.path)}>
                  <span className="ep-title">
                    {f.name}
                    {resume(f.key)}
                  </span>
                  <DifficultyBadge s={summaries[f.key]} />
                </button>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
