const WORDS = [
  "bonjour", "cinéma", "merci", "lumière", "histoire", "rêve", "toujours", "peut-être",
  "regarder", "écouter", "demain", "ensemble", "voyage", "étoile",
];

/** Living background: slow colour fields, drifting French words, and a film-grain finish. */
export default function Backdrop() {
  return (
    <div className="backdrop" aria-hidden="true">
      <div className="orb o1" />
      <div className="orb o2" />
      <div className="orb o3" />
      <div className="beam" />
      <div className="drift">
        {WORDS.map((w, i) => (
          <span
            key={w}
            style={{
              left: ((i * 29 + 6) % 90) + "%",
              top: ((i * 47 + 9) % 86) + "%",
              fontSize: 40 + ((i * 17) % 5) * 18,
              animationDuration: 36 + (i % 5) * 9 + "s",
              animationDelay: -(i * 7) + "s",
            }}
          >
            {w}
          </span>
        ))}
      </div>
      <div className="grain" />
    </div>
  );
}
