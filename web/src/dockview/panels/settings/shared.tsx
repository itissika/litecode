import type { ReactNode } from "react";

import type { AdapterDescriptor } from "../../../api/settings";
import { useSettingsStore, type PersistStatus } from "../../../stores/settingsStore";
import { useTurnStore } from "../../../stores/turnStore";

export function useSettingsSaveBlocked(): boolean {
  return useTurnStore((s) => {
    for (const slice of s.byId.values()) {
      if (slice.runState === "running" || slice.runState === "cancelling") {
        return true;
      }
    }
    return false;
  });
}

export function FieldLabel({
  children,
  required,
}: {
  children: ReactNode;
  required?: boolean;
}) {
  return (
    <label className="mb-0.5 block text-xs font-medium text-(--_dk-text-muted)">
      {children}
      {required ? <span className="text-(--_dk-red-500)"> *</span> : null}
    </label>
  );
}

export function TextInput(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...props}
      className={`settings-input ${props.className ?? ""}`}
    />
  );
}

export function TextArea(props: React.TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return (
    <textarea
      {...props}
      className={`settings-input ${props.className ?? ""}`}
    />
  );
}

const PERSIST_LABEL: Record<PersistStatus, string | null> = {
  idle: null,
  pending: "Saving…",
  saving: "Saving…",
  saved: "Saved",
  invalid: "Fix fields to save",
  error: "Could not save",
};

export function PersistStatusLabel() {
  const status = useSettingsStore((s) => s.persistStatus);
  const text = PERSIST_LABEL[status];
  if (!text) return null;
  return (
    <span
      className={`text-dk-xs ${
        status === "error" || status === "invalid"
          ? "text-(--_dk-red-500)"
          : "text-(--_dk-text-muted)"
      }`}
      aria-live="polite"
    >
      {text}
    </span>
  );
}

export function SectionHeader({
  title,
  children,
  divider,
}: {
  title: string;
  children?: ReactNode;
  divider?: boolean;
}) {
  return (
    <div
      className={`flex items-center justify-between gap-2${divider ? " settings-op-divider" : ""}`}
    >
      <h3 className="settings-section-title">{title}</h3>
      {children ? <div className="flex items-center gap-2 pr-2">{children}</div> : null}
    </div>
  );
}

export function SettingsPageShell({
  title,
  actions,
  children,
}: {
  title: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex h-full flex-col">
      <div className="shrink-0 px-6 py-3.5">
        <SectionHeader title={title}>
          <PersistStatusLabel />
          {actions}
        </SectionHeader>
      </div>
      <div className="mx-4.5 shrink-0 border-t border-(--_dk-line)" />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">{children}</div>
    </div>
  );
}

export function adapterDefaultEndpoint(
  adapters: AdapterDescriptor[],
  adapterId: string,
): string {
  return adapters.find((a) => a.id === adapterId)?.default_endpoint?.trim() ?? "";
}
