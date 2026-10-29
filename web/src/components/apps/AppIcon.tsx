import type { AppView } from "../../api/apps";

export function AppIcon({
  app,
  size = 32,
  className = "",
}: {
  app: Pick<AppView, "icon" | "name">;
  size?: number;
  className?: string;
}) {
  if (app.icon.startsWith("data:image/")) {
    return (
      <img
        src={app.icon}
        alt=""
        width={size}
        height={size}
        className={`apps-icon ${className}`}
        draggable={false}
      />
    );
  }
  return (
    <span
      className={`apps-icon apps-icon-emoji ${className}`}
      style={{ fontSize: Math.round(size * 0.72), width: size, height: size }}
      aria-hidden="true"
    >
      {app.icon || "📦"}
    </span>
  );
}
