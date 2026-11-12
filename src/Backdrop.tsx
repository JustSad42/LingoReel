const BARS = 72;
const CAPTIONS = 11;

/**
 * Living background built from what the app is about: speech as a waveform on the
 * horizon, caption lines that fill in as if being read, and a lit floor receding
 * into the dark. No words, so it suits every language.
 */
export default function Backdrop() {
  return (
    <div className="backdrop" aria-hidden="true">
      <div className="orb o1" />
      <div className="orb o2" />
      <div className="floor" />
      <div className="wave">
        {Array.from({ length: BARS }, (_, i) => {
          // Two overlapping rhythms so the wave never visibly repeats.
          const pace = 1.6 + ((i * 7) % 9) * 0.22;
          return <i key={i} style={{ animationDuration: pace + "s", animationDelay: -((i * 0.137) % pace) + "s" }} />;
        })}
      </div>
      {Array.from({ length: CAPTIONS }, (_, i) => (
        <div
          key={i}
          className={i % 3 === 0 ? "caption alt" : "caption"}
          style={{
            left: ((i * 31 + 5) % 82) + "%",
            width: 90 + ((i * 53) % 7) * 34,
            opacity: 0.55 + ((i * 3) % 4) * 0.12,
            animationDuration: 30 + ((i * 11) % 6) * 6 + "s",
            animationDelay: -(i * 5.3) + "s",
          }}
        >
          <b style={{ animationDuration: 3.2 + (i % 4) * 0.9 + "s", animationDelay: -(i * 0.7) + "s" }} />
        </div>
      ))}
      <div className="grain" />
    </div>
  );
}
