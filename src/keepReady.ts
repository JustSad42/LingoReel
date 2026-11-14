import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, MediaReq, plex, Settings } from "./api";
import { enqueue } from "./queue";

/** How many unwatched episodes of a kept series stay prepared ahead. */
export const KEEP_AHEAD = 3;

/** The next unwatched episodes of every series marked "keep ready", queued when not prepared yet. */
async function topUp(s: Settings) {
  const p = s.plex;
  const ids = Object.keys(s.keepReady || {});
  if (!p || ids.length === 0) return;
  const progress = new Map((await api.progressAll()).map(([k, pos, dur]) => [k, dur > 0 ? pos / dur : 0]));
  // Leave anything already queued (including failures) alone, or a failing episode would retry forever.
  const queued = new Set((await api.queueList()).items.map((i) => i.key));
  const reqs: MediaReq[] = [];
  for (const id of ids) {
    try {
      const eps: any[] = (await plex.get(p, `/library/metadata/${id}/allLeaves`)).Metadata || [];
      const watched = (e: any) => {
        const local = progress.get(`plex-${e.ratingKey}`);
        return local !== undefined ? local > 0.9 : (e.viewCount || 0) > 0;
      };
      // Continue after the last episode watched, skipping specials.
      const regular = eps.filter((e) => (e.parentIndex ?? 1) > 0);
      let last = -1;
      regular.forEach((e, i) => watched(e) && (last = i));
      const ahead = regular.slice(last + 1).filter((e) => !watched(e)).slice(0, KEEP_AHEAD);
      for (const e of ahead) {
        const key = `plex-${e.ratingKey}`;
        if (queued.has(key)) continue;
        const done = await api.prepared(key).catch(() => null);
        if (done && done.lang === s.targetLang) continue;
        reqs.push(await plex.media(p, s, String(e.ratingKey)));
      }
    } catch {
      /* server unreachable or series removed: try again next time */
    }
  }
  if (reqs.length) await enqueue(s, reqs);
}

/** Keeps kept series topped up: at start, every 20 minutes, after watching, and when the list changes. */
export function useKeepReady(settings: Settings, ready: boolean, playing: boolean) {
  const latest = useRef(settings);
  latest.current = settings;
  const running = useRef(false);
  const run = () => {
    if (running.current || !ready) return;
    running.current = true;
    topUp(latest.current).finally(() => (running.current = false));
  };
  const kept = Object.keys(settings.keepReady || {}).join(",");
  useEffect(run, [ready, kept, settings.targetLang, settings.plex?.url]);
  // Leaving the player usually means an episode was just watched.
  useEffect(() => {
    if (!playing) run();
  }, [playing]);
  useEffect(() => {
    const id = window.setInterval(run, 20 * 60 * 1000);
    const un = listen("queue-done", () => window.setTimeout(run, 2000));
    return () => {
      window.clearInterval(id);
      un.then((f) => f());
    };
  }, [ready]);
}
