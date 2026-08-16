interface WaveTextProps {
  text: string;
  className?: string;
}

/** Soft per-character color wave — for lightweight waiting/status lines. */
export function WaveText({ text, className = "" }: WaveTextProps) {
  return (
    <span className={`wait-wave-text inline-flex ${className}`}>
      {Array.from(text).map((char, index) => (
        <span
          key={index}
          className="wait-wave-char"
          style={{ animationDelay: `${index * 0.08}s` }}
        >
          {char === " " ? "\u00a0" : char}
        </span>
      ))}
    </span>
  );
}
