import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, MediaReq, QueueItem, QueueList, Settings } from "./api";

/** Shared view of the background preparation queue, kept current from backend events. */
let snapshot: QueueList = { items: [], paused: false };
const subscribers = new Set<() => void>();
const publish = (next: QueueList) => {
  snapshot = next;
  subscribers.forEach((f) => f());
};
export const refreshQueue = () => api.queueList().then(publish).catch(() => {});

let started = false;
function start() {
  if (started) return;
  started = true;
  refreshQueue();
  listen("queue-changed", refreshQueue);
  listen<{ key: string; stage: string; pct: number }>("prepare-progress", (e) => {
    const { key, stage, pct } = e.payload;
    if (!snapshot.items.some((i) => i.key === key)) return;
    publish({ ...snapshot, items: snapshot.items.map((i) => (i.key === key ? { ...i, stage, pct, state: "working" } : i)) });
  });
}

export function useQueue(): QueueList {
  start();
  return useSyncExternalStore(
    (f) => {
      subscribers.add(f);
      return () => subscribers.delete(f);
    },
    () => snapshot,
  );
}

/** Where each stage sits in the whole job, as [start, span] in percent. */
const SHARE: Record<string, [number, number]> = {
  extract: [0, 20],
  listen: [20, 50],
  align: [70, 5],
  translate: [75, 25],
  done: [100, 0],
};
export function overall(stage: string, pct: number): number {
  const [base, span] = SHARE[stage] || [0, 0];
  return Math.min(100, base + (span * pct) / 100);
}
export const STAGE_TEXT: Record<string, string> = {
  extract: "Reading the file",
  listen: "Listening to the audio",
  align: "Lining up",
  translate: "Translating",
  done: "Finishing",
};

export const optsFor = (s: Settings) => ({
  target_lang: s.targetLang,
  native_lang: s.nativeLang,
  target_sub: "auto",
  native_sub: "auto",
  align: s.align,
});

/** Add videos to the end of the queue; returns how many were new. */
export const enqueue = (s: Settings, reqs: MediaReq[]) =>
  api.queueAdd(reqs.map((req) => ({ req, opts: optsFor(s) })));

export type { QueueItem };
