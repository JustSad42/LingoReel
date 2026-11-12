import { invoke } from "@tauri-apps/api/core";

export type Tok = { t: string; w: boolean; lemma?: string; g?: string; r?: string; s: number; e: number };
export type Cue = { start: number; end: number; text: string; tokens: Tok[]; en: string };
export type Prepared = {
  key: string;
  title: string;
  lang: string;
  cues: Cue[];
  aligned: boolean;
  target_source: string;
  native_source: string;
  audio_index: number | null;
  offset: number;
  note: string;
};
export type SubTrack = {
  id: string;
  lang: string;
  codec: string;
  title: string;
  forced: boolean;
  index?: number | null;
  url?: string | null;
  path?: string | null;
};
export type AudioTrack = { index: number; lang: string; title: string };
export type MediaReq = {
  key: string;
  title: string;
  url: string;
  duration: number;
  subs: SubTrack[];
  audio: AudioTrack[];
  /** Plex rating key, when the item came from Plex. */
  plexId?: string;
  /** Series or folder this belongs to; audio choices are remembered per group. */
  group?: string;
};
export type PrepareOpts = {
  target_lang: string;
  native_lang: string;
  target_sub: string;
  native_sub: string;
  align: boolean;
};
export type Entry = {
  word: string;
  pos: string;
  gender: string;
  ipa: string;
  glosses: string[];
  form_tags: string[];
};
export type Lookup = { word: string; lemma: string | null; rank: number | null; entries: Entry[] };
export type Component = {
  id: string;
  name: string;
  about: string;
  size_mb: number;
  required: boolean;
  installed: boolean;
  note: string;
};
export type UnknownWord = { lemma: string; count: number; rank: number | null; gloss: string; example: string };
export type MediaStats = {
  words: number;
  known: number;
  learning: number;
  unique: number;
  unique_known: number;
  lines: number;
  aligned: boolean;
  unknown: UnknownWord[];
};
export type Card = {
  id: number;
  lang: string;
  lemma: string;
  word: string;
  sentence: string;
  translation: string;
  definition: string;
  media_key: string;
  media_title: string;
  pos: number;
  clip: string;
  image: string;
  created: number;
  due: number;
  state: number;
  reps: number;
  lapses: number;
  stability: number;
  preview: [number, number, number, number];
};
export type MinedLine = {
  cue: number;
  start: number;
  end: number;
  text: string;
  en: string;
  lemma: string;
  word: string;
  gloss: string;
  rank: number | null;
};
export type Occurrence = {
  key: string;
  title: string;
  url: string;
  start: number;
  end: number;
  text: string;
  en: string;
  word: string;
};
export type FileEntry = { path: string; name: string; folder: string; key: string };
export type PlexConn = { name: string; url: string; token: string };

export type Settings = {
  targetLang: string;
  nativeLang: string;
  plex: PlexConn | null;
  clientId: string;
  folders: string[];
  pathFrom: string;
  pathTo: string;
  autoPause: boolean;
  pauseOnHover: boolean;
  /** "show" | "blur" | "hide" */
  english: string;
  slowUnknown: boolean;
  slowSpeed: number;
  align: boolean;
  colorWords: boolean;
  /** "full" | "focus" (known words dimmed) | "listen" (line appears after it is spoken) */
  subMode: string;
  /** Tint verbs by tense. */
  tenseColors: boolean;
  /** "auto" or a zoom factor such as 1.25 */
  uiScale: string;
  /** Plex library keys the user chose not to see. */
  hiddenLibraries: string[];
  volume: number;
  speed: number;
  /** Side panel left open in the player. */
  panel: string;
  /** Library that was open last. */
  lastSource: unknown;
  /** Audio track chosen per series or folder. */
  audioPrefs: Record<string, string>;
  /** Japanese reading aid above each word: "romaji" | "kana" | "none" */
  reading: string;
  subSize: number;
  llmUrl: string;
  llmModel: string;
  setupDone: boolean;
};

export const DEFAULTS: Settings = {
  targetLang: "fr",
  nativeLang: "en",
  plex: null,
  clientId: "",
  folders: [],
  pathFrom: "",
  pathTo: "",
  autoPause: false,
  pauseOnHover: true,
  english: "show",
  slowUnknown: false,
  slowSpeed: 0.8,
  align: true,
  colorWords: true,
  subMode: "full",
  tenseColors: true,
  uiScale: "auto",
  hiddenLibraries: [],
  volume: 100,
  speed: 1,
  panel: "none",
  lastSource: null,
  audioPrefs: {},
  reading: "romaji",
  subSize: 34,
  llmUrl: "",
  llmModel: "",
  setupDone: false,
};

/** Word status: 0 new, 1 learning, 2 known, 3 ignored (names, noise). */
export const STATUS = ["New", "Learning", "Known", "Ignored"];

export const api = {
  info: () => invoke<{ version: string; dataDir: string; nvidia: boolean }>("app_info"),
  settingsGet: () => invoke<Partial<Settings>>("settings_get"),
  settingsSet: (value: Settings) => invoke("settings_set", { value }),
  wordsAll: (lang: string) => invoke<[string, number][]>("words_all", { lang }),
  wordsSet: (lang: string, lemmas: string[], status: number) => invoke("words_set", { lang, lemmas, status }),
  progressSet: (key: string, title: string, position: number, duration: number) =>
    invoke("progress_set", { key, title, position, duration }),
  progressAll: () => invoke<[string, number, number][]>("progress_all"),
  lookup: (lang: string, word: string) => invoke<Lookup>("dict_lookup", { lang, word }),
  phrase: (lang: string, phrase: string) => invoke<Entry[]>("dict_phrase", { lang, phrase }),
  topLemmas: (lang: string, count: number) => invoke<string[]>("dict_top_lemmas", { lang, count }),
  components: () => invoke<Component[]>("components_status"),
  install: (id: string) => invoke("component_install", { id }),
  remove: (id: string) => invoke("component_remove", { id }),
  mpvStart: () => invoke("mpv_start"),
  mpv: (...args: unknown[]) => invoke("mpv_cmd", { args }),
  mpvStop: () => invoke("mpv_stop"),
  prepare: (req: MediaReq, opts: PrepareOpts) => invoke<Prepared>("prepare_media", { req, opts }),
  prepared: (key: string) => invoke<Prepared | null>("prepared_get", { key }),
  preparedDelete: (key: string) => invoke("prepared_delete", { key }),
  stats: (key: string) => invoke<MediaStats | null>("media_stats", { key }),
  summaries: (keys: string[]) => invoke<Record<string, MediaStats>>("media_summaries", { keys }),
  cacheGet: (key: string) => invoke<string | null>("cache_get", { key }),
  cacheSet: (key: string, value: string) => invoke("cache_set", { key, value }),
  mine: (key: string) => invoke<MinedLine[]>("media_mine", { key }),
  occurrences: (lang: string, lemma: string, skipKey: string, skipStart: number) =>
    invoke<Occurrence[]>("word_occurrences", { lang, lemma, skipKey, skipStart }),
  scanFolder: (path: string) => invoke<FileEntry[]>("scan_folder", { path }),
  probeFile: (path: string) => invoke<MediaReq>("probe_file", { path }),
  mapPath: (file: string, from: string, to: string) => invoke<string | null>("map_path", { file, from, to }),
  cardAdd: (card: Record<string, unknown>) => invoke<number>("card_add", { card }),
  cardsDue: (lang: string, limit: number) => invoke<Card[]>("cards_due", { lang, limit }),
  cardsList: (lang: string) => invoke<Card[]>("cards_list", { lang }),
  cardGrade: (id: number, rating: number) => invoke("card_grade", { id, rating }),
  cardDelete: (id: number) => invoke("card_delete", { id }),
  cardsCounts: (lang: string) =>
    invoke<{ due: number; new: number; total: number; reviewed_today: number }>("cards_counts", { lang }),
  llmStatus: () => invoke<{ available: boolean; managed: boolean }>("llm_status"),
  explain: (lang: string, text: string, context: string, mode: "explain" | "translate") =>
    invoke<string>("llm_explain", { lang, text, context, mode }),
  shadow: (audio: number[], expected: string, lang: string) =>
    invoke<{ heard: string; words: [string, boolean][]; score: number }>("shadow_score", { audio, expected, lang }),
  http: (method: string, url: string, headers: Record<string, string> = {}, timeoutMs = 20000) =>
    invoke<any>("http_json", { method, url, headers, timeoutMs }),
};

export const LANGS: Record<string, string> = {
  fr: "French",
  en: "English",
  es: "Spanish",
  de: "German",
  it: "Italian",
  pt: "Portuguese",
  ja: "Japanese",
};
/** Languages that can be studied, with the name shown in Settings. */
export const LEARN_LANGS: [string, string][] = [
  ["fr", "French"],
  ["es", "Spanish (Castellano)"],
  ["de", "German"],
  ["it", "Italian"],
  ["pt", "Portuguese"],
  ["ja", "Japanese"],
];


/** Verb tenses shown by colour. Unlisted codes (present, infinitive, participles) stay uncoloured. */
export const TENSES: Record<string, string> = {
  pc: "Passé composé",
  imp: "Imparfait",
  pqp: "Plus-que-parfait",
  ps: "Passé simple",
  fut: "Futur",
  fp: "Futur proche",
  cond: "Conditionnel",
  subj: "Subjonctif",
  imper: "Impératif",
};
export const TENSE_NAMES: Record<string, string> = {
  ...TENSES,
  pres: "Présent",
  inf: "Infinitif",
  pp: "Participe passé",
  ppr: "Participe présent",
};

const TENSE_LABELS: Record<string, Record<string, string>> = {
  fr: TENSE_NAMES,
  es: {
    pc: "Pretérito perfecto", imp: "Imperfecto", pqp: "Pluscuamperfecto", ps: "Pretérito indefinido", fut: "Futuro",
    fp: "Ir a + infinitivo", cond: "Condicional", subj: "Subjuntivo", imper: "Imperativo", pres: "Presente",
    inf: "Infinitivo", pp: "Participio", ppr: "Gerundio",
  },
  it: {
    pc: "Passato prossimo", imp: "Imperfetto", pqp: "Trapassato prossimo", ps: "Passato remoto", fut: "Futuro",
    cond: "Condizionale", subj: "Congiuntivo", imper: "Imperativo", pres: "Presente", inf: "Infinito",
    pp: "Participio passato", ppr: "Gerundio",
  },
  pt: {
    pc: "Pretérito perfeito composto", imp: "Imperfeito", pqp: "Mais-que-perfeito", ps: "Pretérito perfeito",
    fut: "Futuro", fp: "Ir + infinitivo", cond: "Condicional", subj: "Subjuntivo", imper: "Imperativo",
    pres: "Presente", inf: "Infinitivo", pp: "Particípio", ppr: "Gerúndio",
  },
};
/** Tense names for a language, or null when tenses are not analysed for it. */
export const tenseNames = (lang: string): Record<string, string> | null => TENSE_LABELS[lang] || null;

// ---------- Japanese readings ----------
const KANA: Record<string, string> = {
  あ: "a", い: "i", う: "u", え: "e", お: "o", か: "ka", き: "ki", く: "ku", け: "ke", こ: "ko",
  さ: "sa", し: "shi", す: "su", せ: "se", そ: "so", た: "ta", ち: "chi", つ: "tsu", て: "te", と: "to",
  な: "na", に: "ni", ぬ: "nu", ね: "ne", の: "no", は: "ha", ひ: "hi", ふ: "fu", へ: "he", ほ: "ho",
  ま: "ma", み: "mi", む: "mu", め: "me", も: "mo", や: "ya", ゆ: "yu", よ: "yo",
  ら: "ra", り: "ri", る: "ru", れ: "re", ろ: "ro", わ: "wa", ゐ: "i", ゑ: "e", を: "o", ん: "n",
  が: "ga", ぎ: "gi", ぐ: "gu", げ: "ge", ご: "go", ざ: "za", じ: "ji", ず: "zu", ぜ: "ze", ぞ: "zo",
  だ: "da", ぢ: "ji", づ: "zu", で: "de", ど: "do", ば: "ba", び: "bi", ぶ: "bu", べ: "be", ぼ: "bo",
  ぱ: "pa", ぴ: "pi", ぷ: "pu", ぺ: "pe", ぽ: "po", ぁ: "a", ぃ: "i", ぅ: "u", ぇ: "e", ぉ: "o", ゔ: "vu",
};
const SMALL_Y: Record<string, string> = { ゃ: "a", ゅ: "u", ょ: "o" };

/** Kana (hiragana or katakana) to Hepburn romaji. Anything else passes through. */
export function toRomaji(kana: string): string {
  const h = [...kana].map((c) => {
    const code = c.codePointAt(0) as number;
    return code >= 0x30a1 && code <= 0x30f6 ? String.fromCodePoint(code - 0x60) : c;
  });
  let out = "";
  let double = false;
  for (let i = 0; i < h.length; i++) {
    const c = h[i];
    if (c === "っ") {
      double = true;
      continue;
    }
    if (c === "ー") {
      const last = out.slice(-1);
      if ("aiueo".includes(last)) out += last;
      continue;
    }
    let r = KANA[c];
    if (r === undefined) {
      out += c;
      double = false;
      continue;
    }
    const y = SMALL_Y[h[i + 1]];
    if (y && r.endsWith("i") && r.length > 1) {
      // きゃ -> kya, しゃ -> sha, ちゃ -> cha, じゃ -> ja
      const stem = r.slice(0, -1);
      r = ["sh", "ch", "j"].includes(stem) ? stem + y : stem + "y" + y;
      i++;
    }
    if (double) {
      r = (r.startsWith("ch") ? "t" : r[0]) + r;
      double = false;
    }
    out += r;
  }
  return out;
}

/** Japanese dictation: accept romaji or kana, compare sound by sound. */
function gradeJapanese(tokens: Tok[], typed: string): { marks: Mark[]; hits: number } {
  const words = tokens.filter((k) => k.w);
  const clean = (x: string) => toRomaji(x).toLowerCase().replace(/[^a-z]/g, "");
  const exp = words.map((k) => clean(k.r || k.t));
  const all = exp.join("");
  const got = clean(typed);
  const n = all.length;
  const m = got.length;
  const dp = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = all[i] === got[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const ok = new Array<boolean>(n).fill(false);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (all[i] === got[j]) {
      ok[i] = true;
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
    else j++;
  }
  let at = 0;
  const marks: Mark[] = words.map((k, wi) => {
    const len = exp[wi].length;
    const right = ok.slice(at, at + len).filter(Boolean).length;
    at += len;
    const state = len === 0 || right === len ? "hit" : right / len >= 0.7 ? "close" : "miss";
    return { w: k.t, state };
  });
  return { marks, hits: marks.filter((x) => x.state !== "miss").length };
}

export type Mark = { w: string; state: "hit" | "close" | "miss" };

const plain = (x: string) => x.normalize("NFD").replace(/[\u0300-\u036f]/g, "");
const parts = (x: string) => x.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);

/** Dictation: compare what was typed with the words of the line. */
export function gradeDictation(tokens: Tok[], typed: string, lang = ""): { marks: Mark[]; hits: number } {
  if (lang === "ja") return gradeJapanese(tokens, typed);
  const words = tokens.filter((k) => k.w);
  const exp: { tok: number; p: string }[] = [];
  words.forEach((k, i) => parts(k.t).forEach((p) => exp.push({ tok: i, p })));
  const got = parts(typed);
  const n = exp.length;
  const m = got.length;
  const same = (a: string, b: string) => (a === b ? 2 : plain(a) === plain(b) ? 1 : 0);
  // Longest common subsequence, so a missed word does not shift everything after it.
  const dp = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = same(exp[i].p, got[j]) ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const quality = new Array<number>(n).fill(0);
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    const q = same(exp[i].p, got[j]);
    if (q && dp[i][j] === dp[i + 1][j + 1] + 1) {
      quality[i] = q;
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
    else j++;
  }
  const marks: Mark[] = words.map((k, wi) => {
    const qs = exp.map((e, idx) => (e.tok === wi ? quality[idx] : -1)).filter((q) => q >= 0);
    const state = qs.length && qs.every((q) => q === 2) ? "hit" : qs.length && qs.every((q) => q >= 1) ? "close" : "miss";
    return { w: k.t, state };
  });
  return { marks, hits: marks.filter((x) => x.state !== "miss").length };
}

export function fmtTime(s: number): string {
  if (!isFinite(s) || s < 0) s = 0;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = Math.floor(s % 60);
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return (h > 0 ? h + ":" : "") + mm + ":" + String(sec).padStart(2, "0");
}

export function fmtInterval(seconds: number): string {
  if (seconds < 3600) return Math.max(1, Math.round(seconds / 60)) + " min";
  if (seconds < 86400) return Math.round(seconds / 3600) + " h";
  const d = Math.round(seconds / 86400);
  if (d < 31) return d + " d";
  if (d < 365) return Math.round(d / 30) + " mo";
  return (d / 365).toFixed(1) + " y";
}

/** How common a word is, from its frequency rank in film and TV subtitles. */
export function freqLabel(rank: number | null): { label: string; level: number } | null {
  if (!rank) return null;
  if (rank <= 500) return { label: "Essential (top 500)", level: 4 };
  if (rank <= 2000) return { label: "Very common (top 2,000)", level: 3 };
  if (rank <= 5000) return { label: "Common (top 5,000)", level: 2 };
  if (rank <= 15000) return { label: "Less common", level: 1 };
  return { label: "Rare", level: 0 };
}

// ---------- Plex ----------

const plexHeaders = (s: Settings, token?: string): Record<string, string> => ({
  "X-Plex-Client-Identifier": s.clientId,
  "X-Plex-Product": "LingoReel",
  "X-Plex-Version": "1.0",
  "X-Plex-Platform": "Windows",
  "X-Plex-Device-Name": "LingoReel",
  ...(token ? { "X-Plex-Token": token } : {}),
});

export const plex = {
  async pinCreate(s: Settings): Promise<{ id: number; code: string; url: string }> {
    const pin = await api.http("POST", "https://plex.tv/api/v2/pins?strong=true", plexHeaders(s));
    const url =
      "https://app.plex.tv/auth#?clientID=" +
      encodeURIComponent(s.clientId) +
      "&code=" +
      encodeURIComponent(pin.code) +
      "&context%5Bdevice%5D%5Bproduct%5D=LingoReel";
    return { id: pin.id, code: pin.code, url };
  },
  async pinToken(s: Settings, id: number): Promise<string | null> {
    const pin = await api.http("GET", `https://plex.tv/api/v2/pins/${id}`, plexHeaders(s));
    return pin?.authToken || null;
  },
  /** Find the user's servers and a connection to each that works from this PC. */
  async servers(s: Settings, token: string): Promise<PlexConn[]> {
    const res: any[] = await api.http(
      "GET",
      "https://plex.tv/api/v2/resources?includeHttps=1&includeRelay=0",
      plexHeaders(s, token),
    );
    const out: PlexConn[] = [];
    for (const r of res || []) {
      if (!String(r.provides || "").includes("server")) continue;
      const tok = r.accessToken || token;
      const conns: any[] = [...(r.connections || [])].sort((a, b) => Number(b.local) - Number(a.local));
      const candidates: string[] = [];
      for (const c of conns) {
        if (c.local) candidates.push(`http://${c.address}:${c.port}`);
        candidates.push(c.uri);
      }
      for (const url of candidates) {
        try {
          await api.http("GET", url + "/identity", { "X-Plex-Token": tok }, 4000);
          out.push({ name: r.name, url, token: tok });
          break;
        } catch {
          /* try the next address */
        }
      }
    }
    return out;
  },
  get(p: PlexConn, path: string): Promise<any> {
    return api.http("GET", p.url + path, { "X-Plex-Token": p.token }).then((r) => r?.MediaContainer ?? {});
  },
  thumb(p: PlexConn, thumb: string | undefined, w = 240, h = 360): string {
    if (!thumb) return "";
    return (
      `${p.url}/photo/:/transcode?width=${w}&height=${h}&minSize=1&upscale=1&url=` +
      encodeURIComponent(thumb) +
      `&X-Plex-Token=${p.token}`
    );
  },
  /** Everything needed to play and prepare one movie or episode. */
  async media(p: PlexConn, s: Settings, ratingKey: string): Promise<MediaReq> {
    const mc = await plex.get(p, `/library/metadata/${ratingKey}`);
    const md = mc.Metadata?.[0];
    const part = md?.Media?.[0]?.Part?.[0];
    if (!part) throw new Error("Plex has no playable file for this item.");
    const streams: any[] = part.Stream || [];
    const mapped = part.file ? await api.mapPath(part.file, s.pathFrom, s.pathTo) : null;
    const withToken = (path: string) => `${p.url}${path}${path.includes("?") ? "&" : "?"}X-Plex-Token=${p.token}`;
    const title =
      md.type === "episode"
        ? `${md.grandparentTitle} · S${md.parentIndex}E${md.index} · ${md.title}`
        : `${md.title}${md.year ? ` (${md.year})` : ""}`;
    return {
      key: `plex-${ratingKey}`,
      plexId: String(ratingKey),
      group: `plex-${md.grandparentRatingKey || ratingKey}`,
      title,
      url: mapped ?? withToken(part.key),
      duration: (md.duration || 0) / 1000,
      subs: streams
        .filter((x) => x.streamType === 3)
        .map((x) => ({
          id: `p${x.id}`,
          lang: x.languageTag || x.languageCode || "",
          codec: x.codec || "",
          title: x.displayTitle || x.title || "",
          forced: !!x.forced,
          index: x.key ? null : x.index,
          url: x.key ? withToken(x.key) : null,
        })),
      audio: streams
        .filter((x) => x.streamType === 2)
        .map((x) => ({ index: x.index, lang: x.languageTag || x.languageCode || "", title: x.displayTitle || "" })),
    };
  },
};
