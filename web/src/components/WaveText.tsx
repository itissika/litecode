interface WaveTextProps {
  text: string;
  className?: string;
  /** Per-character span class (defaults to the wait-shell wave). */
  charClass?: string;
}

/** Soft per-character color wave — for lightweight waiting/status lines. */
export function WaveText({
  text,
  className = "",
  charClass = "wait-wave-char",
}: WaveTextProps) {
  return (
    <span className={`wait-wave-text inline-flex ${className}`}>
      {Array.from(text).map((char, index) => (
        <span
          key={index}
          className={charClass}
          style={{ animationDelay: `${index * 0.08}s` }}
        >
          {char === " " ? "\u00a0" : char}
        </span>
      ))}
    </span>
  );
}
