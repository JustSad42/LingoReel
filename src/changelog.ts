/** What changed in each version, newest first. Shown once after an update. */
export type Release = { version: string; title: string; items: [string, string][] };

export const CHANGELOG: Release[] = [
  {
    version: "0.7.0",
    title: "Study tools",
    items: [
      ["Set phrases", "Expressions such as « en train de » or « sin embargo » are dotted underneath. Click a word in one to see what the whole phrase means."],
      ["Conjugation tables", "Click a verb and choose Conjugate to see every form, with the one you heard highlighted."],
      ["Grammar notes", "Click a tense in the legend or in a word's popup: what it is for, how it is built, and lines from your own videos that use it."],
      ["Word families", "The popup lists related words built on the same root."],
      ["Gender colours", "Nouns can be tinted blue for masculine and pink for feminine (C, or the Genders chip)."],
      ["Loop a line", "B loops the current line, Shift+B loops any stretch you mark, and Slow plays the loop slower."],
      ["Smarter auto-pause", "P now cycles: pause after every line, only after lines with a word you are learning, or never."],
      ["Back with English", "Q jumps back to the start of the line and shows its English, even when English is hidden."],
      ["Subtitle style", "Move the subtitles up or down (Shift+↑/↓), change the font, the box behind them, or add an outline in Settings."],
      ["New review cards", "Fill in the missing word, or listen first and read after. Choose in Settings or mix them all."],
      ["For your level", "The Library suggests prepared videos where you already know most of the words."],
      ["Better model", "An optional larger model (Gemma 3 12B) gives better translations and explanations on cards with enough memory."],
      ["Light setup", "Start with a smaller download and add the rest later."],
    ],
  },
  {
    version: "0.6.2",
    title: "English voice",
    items: [["English voice", "Mixed subtitles and review-card translations can be read aloud in English."]],
  },
  {
    version: "0.6.0",
    title: "Reading aloud and mixed subtitles",
    items: [
      ["Read aloud", "An offline voice reads words, lines and review cards."],
      ["Mixed subtitles", "The English line with the words you know shown in the language you study."],
      ["Beginner translations", "Simple, short translations when the local model has to translate."],
      ["Subtitles through Plex", "Missing subtitles are requested from your Plex server first."],
      ["Keep ready", "The next 3 unwatched episodes of a series are always prepared."],
    ],
  },
];

/** Releases newer than the one last seen, up to the running version. */
export function changesSince(last: string, current: string): Release[] {
  const num = (v: string) => v.split(".").map((x) => parseInt(x) || 0);
  const cmp = (a: string, b: string) => {
    const [x, y] = [num(a), num(b)];
    for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) - (y[i] || 0);
    return 0;
  };
  return CHANGELOG.filter((r) => cmp(r.version, current) <= 0 && (!last || cmp(r.version, last) > 0)).slice(0, last ? undefined : 1);
}
