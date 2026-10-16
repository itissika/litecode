import type { ReactNode } from "react";

import { ToolInfoIcon } from "./InfoIcon";
import type { MetaField } from "./paramMeta";

/** Shared file-path header row for file tools (read / write / edit). */
export function FilePathHeader({
  icon,
  path,
  fallback,
  colorClass,
  meta,
  children,
}: {
  icon: ReactNode;
  path: string | undefined;
  fallback: string;
  colorClass: string;
  meta: MetaField[];
  children?: ReactNode;
}) {
  return (
    <div
      className={`flex items-center gap-1.5 text-dk-xs font-medium ${colorClass}`}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate font-mono">
        {path ?? fallback}
      </span>
      {children}
      <ToolInfoIcon fields={meta} />
    </div>
  );
}
