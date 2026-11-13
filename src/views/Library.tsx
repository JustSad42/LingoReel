import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, FileEntry, fmtTime, MediaReq, MediaStats, plex, PlexConn } from "../api";
import { enqueue, overall, useQueue } from "../queue";
import { useApp } from "../App";
import Icon from "../Icon";

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

/** Listings already loaded this session, for instant switching between libraries. */
const memory = new Map<string, unknown>();

export default function Library({ active }: { active: boolean }) {
  const { settings, save, toast, play } = useApp();
  const p = settings.plex;
  const [sections, setSections] = useState<Item[]>([]);
  const [source, setSource] = useState<Source | null>((settings.lastSource as Source) ?? null);
  const [managing, setManaging] = useState(false);
  const [choices, setChoices] = useState<PlexConn[]>([]);
  const request = useRef(0);
  const hidden = settings.hiddenLibraries;

  /** Show what was saved last time straight away, then refresh from the server in the background. */
  const cached = async <T,>(key: string, fetcher: () => Promise<T>, apply: (v: T) => void) => {
    const id = ++request.current;
    let had = false;
    if (memory.has(key)) {
      apply(memory.get(key) as T);
      had = true;
    } else {
      try {
        const raw = await api.cacheGet(key);
        if (raw && id === request.current) {
          apply(JSON.parse(raw));
          had = true;
        }
      } catch {
        /* no cache yet */
      }
    }
    if (!had) setLoading(true);
    try {
      const fresh = await fetcher();
      if (id === request.current) {
        apply(fresh);
        setError("");
      }
      memory.set(key, fresh);
      api.cacheSet(key, JSON.stringify(fresh));
    } catch (e) {
      if (!had && id === request.current) setError(String(e));
    }
    if (id === request.current) setLoading(false);
  };
  const slim = (list: Item[]): Item[] =>
    (list || []).map((m) => ({
      ratingKey: m.ratingKey, title: m.title, thumb: m.thumb, year: m.year, type: m.type, leafCount: m.leafCount,
      viewOffset: m.viewOffset, duration: m.duration, parentIndex: m.parentIndex, index: m.index,
    }));

  useEffect(() => {
    save({ lastSource: source });
  }, [source]);
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
  const queue = useQueue();
  const queued = useMemo(() => new Map(queue.items.map((i) => [i.key, i])), [queue]);

  // Coming back from the player or another screen: pick up new watch progress and difficulty scores.
  useEffect(() => {
    if (!active) return;
    api.progressAll().then((rows) => setProgress(new Map(rows.map(([k, pos, dur]) => [k, [pos, dur]]))));
    refreshSummaries();
  }, [active]);

  useEffect(() => {
    if (!p) {
      setSections([]);
      return;
    }
    const applySections = (dirs: Item[]) => {
      setSections(dirs);
      const first = dirs.find((d) => !hidden.includes(d.key));
      setSource((cur) => cur ?? (first ? { kind: "plex", key: first.key, title: first.title, type: first.type } : null));
    };
    const key = `plex:${p.url}:sections`;
    api
      .cacheGet(key)
      .then((raw) => raw && applySections(JSON.parse(raw)))
      .catch(() => {});
    plex
      .get(p, "/library/sections")
      .then((mc) => {
        const dirs = (mc.Directory || [])
          .filter((d: Item) => d.type === "movie" || d.type === "show")
          .map((d: Item) => ({ key: d.key, title: d.title, type: d.type }));
        applySections(dirs);
        setError("");
        api.cacheSet(key, JSON.stringify(dirs));
      })
      .catch((e) => setError(`Could not reach your Plex server at ${p.url}. Showing what was saved last time. ${e}`));
  }, [p?.url, p?.token]);

  useEffect(() => {
    if (!source) return;
    setShow(null);
    setQuery("");
    if (source.kind === "plex" && p) {
      if (!memory.has(`plex:${p.url}:section:${source.key}`)) setItems([]);
      cached(`plex:${p.url}:section:${source.key}`, () => plex.get(p, `/library/sections/${source.key}/all`).then((mc) => slim(mc.Metadata)), setItems);
    } else if (source.kind === "folder") {
      if (!memory.has(`folder:${source.path}`)) setFiles([]);
      cached(`folder:${source.path}`, () => api.scanFolder(source.path), setFiles);
    }
  }, [source]);

  useEffect(() => {
    if (!show || !p) return;
    if (!memory.has(`plex:${p.url}:show:${show.ratingKey}`)) setEpisodes([]);
    cached(`plex:${p.url}:show:${show.ratingKey}`, () => plex.get(p, `/library/metadata/${show.ratingKey}/allLeaves`).then((mc) => slim(mc.Metadata)), setEpisodes);
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
  // Something left the queue (finished): show its difficulty.
  const queuedKeys = queue.items.map((i) => i.key).join("|");
  useEffect(refreshSummaries, [queuedKeys]);

  /** Queue state when the video is waiting or being prepared, otherwise how hard it is. */
  const badge = (key: string) => {
    const q = queued.get(key);
    if (!q) return <DifficultyBadge s={summaries[key]} />;
    if (q.state === "working") return <span className="qstate working">Preparing {Math.round(overall(q.stage, q.pct))}%</span>;
    if (q.state === "failed") return <span className="qstate failed" title={q.error}>Could not prepare</span>;
    return <span className="qstate">In queue</span>;
  };
  const queueButton = (title: string, add: () => void) => (
    <span
      role="button"
      className="queue-btn"
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        add();
      }}
    >
      <Icon name="queue-add" size={16} />
    </span>
  );
  const report = (added: number, asked: number) =>
    toast(
      added
        ? `Added ${added === 1 ? "1 video" : added + " videos"} to the queue. They are prepared in the background.`
        : asked
          ? "Already prepared or already being prepared."
          : "Nothing to add.",
    );
  const queuePlex = async (ratingKeys: string[]) => {
    if (!p) return;
    if (ratingKeys.length > 3) toast(`Adding ${ratingKeys.length} episodes to the queue…`);
    const reqs: MediaReq[] = [];
    for (const k of ratingKeys) {
      try {
        reqs.push(await plex.media(p, settings, k));
      } catch (e) {
        toast(String(e));
      }
    }
    report(await enqueue(settings, reqs), ratingKeys.length);
  };
  const queueShow = async (item: Item) => {
    if (!p) return;
    const eps: Item[] = (await plex.get(p, `/library/metadata/${item.ratingKey}/allLeaves`)).Metadata || [];
    queuePlex(eps.map((e) => String(e.ratingKey)));
  };
  const queueFiles = async (paths: string[]) => {
    if (paths.length > 3) toast(`Adding ${paths.length} videos to the queue…`);
    const reqs: MediaReq[] = [];
    for (const path of paths) {
      try {
        reqs.push({ ...(await api.probeFile(path)), group: "folder-" + path.replace(/[\\/][^\\/]*$/, "") });
      } catch (e) {
        toast(String(e));
      }
    }
    report(await enqueue(settings, reqs), paths.length);
  };

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
      if (servers.length > 1) {
        // Own server plus servers shared by friends: let the person pick.
        setChoices(servers);
        toast("You have access to more than one Plex server. Choose which one to use.");
      } else {
        save({ plex: servers[0] });
        toast(`Connected to ${servers[0].name}`);
      }
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
      .then((req) => play({ ...req, group: "folder-" + path.replace(/[\\/][^\\/]*$/, "") }))
      .catch((e) => toast(String(e)));

  const playPlex = (ratingKey: string) =>
    plex
      .media(p as PlexConn, settings, ratingKey)
      .then(play)
      .catch((e) => toast(String(e)));

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
        <div className="side-head">
          <h3>Plex</h3>
          {p && sections.length > 0 && (
            <button className="ghost small" onClick={() => setManaging(!managing)} title="Choose which libraries to show">
              {managing ? "Done" : "Edit"}
            </button>
          )}
        </div>
        {p ? (
          <>
            {sections
              .filter((s) => managing || !hidden.includes(s.key))
              .map((s) => {
                const isHidden = hidden.includes(s.key);
                return (
                  <div key={s.key} className="side-row">
                    <button
                      className={
                        (source?.kind === "plex" && source.key === s.key ? "side-item active" : "side-item") + (isHidden ? " dimmed" : "")
                      }
                      onClick={() => !isHidden && setSource({ kind: "plex", key: s.key, title: s.title, type: s.type })}
                    >
                      <Icon name={s.type === "show" ? "tv" : "film"} size={17} />
                      <span>{s.title}</span>
                    </button>
                    {managing && (
                      <button
                        className="icon-btn small"
                        title={isHidden ? "Show this library" : "Hide this library"}
                        onClick={() => {
                          const next = isHidden ? hidden.filter((k) => k !== s.key) : [...hidden, s.key];
                          save({ hiddenLibraries: next });
                          if (!isHidden && source?.kind === "plex" && source.key === s.key) {
                            const other = sections.find((x) => !next.includes(x.key));
                            setSource(other ? { kind: "plex", key: other.key, title: other.title, type: other.type } : null);
                          }
                        }}
                      >
                        <Icon name={isHidden ? "eye-off" : "eye"} size={16} />
                      </button>
                    )}
                  </div>
                );
              })}
            {!managing && hidden.length > 0 && <div className="muted small-text side-note">{hidden.length} hidden</div>}
            <div className="server-chip">
              <i /> {p.name}
            </div>
          </>
        ) : (
          choices.length > 0 ? (
            <>
              <div className="muted small-text side-note">Choose a server</div>
              {choices.map((c) => (
                <button
                  key={c.url}
                  className="side-item"
                  onClick={() => {
                    save({ plex: c, lastSource: null });
                    setSource(null);
                    setChoices([]);
                    toast(`Connected to ${c.name}`);
                  }}
                >
                  <Icon name="server" size={17} />
                  <span>{c.name}</span>
                </button>
              ))}
            </>
          ) : (
            <button className="primary small" disabled={linking} onClick={connectPlex}>
              {linking ? "Waiting for sign-in in your browser…" : "Connect Plex"}
            </button>
          )
        )}
        <h3>Folders</h3>
        {settings.folders.map((f) => (
          <button
            key={f}
            className={source?.kind === "folder" && source.path === f ? "side-item active" : "side-item"}
            onClick={() => setSource({ kind: "folder", path: f })}
            title={f}
          >
            <Icon name="folder" size={17} />
            <span>{f.split(/[\\/]/).filter(Boolean).pop()}</span>
          </button>
        ))}
        <button className="side-item subtle" onClick={addFolder}>
          <Icon name="plus" size={17} />
          <span>Add folder</span>
        </button>
        <button className="side-item subtle" onClick={openFile}>
          <Icon name="film" size={17} />
          <span>Open a video file</span>
        </button>
      </aside>

      <section className="content">
        {error && <div className="error">{error}</div>}
        {!source && !error && (
          <div className="empty">
            <div className="empty-art">
              <Icon name="film" size={34} />
            </div>
            <h2>Add your library</h2>
            <p className="muted">Connect your Plex server, or add a folder of videos, to start watching.</p>
          </div>
        )}

        {source && !show && (
          <div className="row head">
            <h1 title={source.kind === "folder" ? source.path : undefined}>
              {source.kind === "plex" ? source.title : source.path.split(/[\\/]/).filter(Boolean).pop()}
            </h1>
            <div className="spacer" />
            {source.kind === "plex" && (
              <label className="search">
                <Icon name="search" size={16} />
                <input placeholder="Search this library" value={query} onChange={(e) => setQuery(e.target.value)} />
              </label>
            )}
            {source.kind === "folder" && files.length > 0 && (
              <button className="ghost small" onClick={() => queueFiles(files.map((f) => f.path))}>
                <Icon name="queue-add" size={15} /> Queue all
              </button>
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
        {loading && (
          <div className="grid">
            {Array.from({ length: 12 }, (_, i) => (
              <div key={i} className="poster skeleton">
                <div className="poster-img" />
              </div>
            ))}
          </div>
        )}

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
                    {i.thumb && <img loading="lazy" src={plex.thumb(p, i.thumb)} alt="" onLoad={(e) => e.currentTarget.classList.add("loaded")} />}
                    <div className="poster-play">
                      <Icon name={i.type === "show" ? "library" : "play"} size={22} />
                    </div>
                    {i.type !== "show" && resume(key, i.viewOffset, i.duration)}
                    {i.type === "show"
                      ? queueButton("Prepare every episode in the background", () => queueShow(i))
                      : queueButton("Prepare in the background", () => queuePlex([String(i.ratingKey)]))}
                  </div>
                  <div className="poster-title">{i.title}</div>
                  <div className="muted small-text">
                    {i.year}
                    {i.type === "show" && i.leafCount ? ` · ${i.leafCount} episodes` : ""}
                  </div>
                  {badge(key)}
                </button>
              );
            })}
          </div>
        )}

        {show && p && (
          <div>
            <div className="row head">
              <button className="icon-btn" onClick={() => setShow(null)} title="Back">
                <Icon name="back" />
              </button>
              <h1>{show.title}</h1>
              <div className="spacer" />
              <span className="muted small-text">Queue episodes to have them ready, and see how hard each one is.</span>
              <button className="ghost small" onClick={() => queuePlex(episodes.map((e) => String(e.ratingKey)))}>
                <Icon name="queue-add" size={15} /> Queue all
              </button>
            </div>
            {seasons.map(([num, eps]) => (
              <div key={num} className="season">
                <div className="row">
                  <h2>{num === 0 ? "Specials" : `Season ${num}`}</h2>
                  <div className="spacer" />
                  <button className="ghost small" onClick={() => queuePlex(eps.map((e) => String(e.ratingKey)))}>
                    <Icon name="queue-add" size={15} /> Queue season
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
                      {badge(key)}
                      <span className="muted">{fmtTime((e.duration || 0) / 1000)}</span>
                      {queueButton("Prepare in the background", () => queuePlex([String(e.ratingKey)]))}
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
                  {badge(f.key)}
                  {queueButton("Prepare in the background", () => queueFiles([f.path]))}
                </button>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
