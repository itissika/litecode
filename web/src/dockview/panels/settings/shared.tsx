import type { FormEvent, ReactNode } from "react";
import { FloppyDisk } from "@phosphor-icons/react";

import type { AdapterDescriptor } from "../../../api/settings";

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

export function SaveButton({
  disabled,
  saving,
  label = "Save",
  icon,
}: {
  disabled?: boolean;
  saving?: boolean;
  label?: string;
  icon?: React.ReactNode;
}) {
  return (
    <button
      type="submit"
      disabled={disabled || saving}
      className={icon ? "btn-primary btn-icon" : "btn-primary"}
      aria-label={icon ? label : undefined}
      title={icon ? label : undefined}
    >
      {icon ? icon : saving ? "Saving…" : label}
    </button>
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
  save,
  onSubmit,
  children,
}: {
  title: string;
  actions?: ReactNode;
  save?: {
    disabled?: boolean;
    saving?: boolean;
    label?: string;
  };
  onSubmit?: (e: FormEvent) => void;
  children: ReactNode;
}) {
  const chrome = (
    <>
      <div className="shrink-0 px-6 py-3.5">
        <SectionHeader title={title}>
          {actions}
          {save ? (
            <SaveButton
              disabled={save.disabled}
              saving={save.saving}
              label={save.label ?? "Save"}
              icon={<FloppyDisk size={16} />}
            />
          ) : null}
        </SectionHeader>
      </div>
      <div className="mx-4.5 shrink-0 border-t border-(--_dk-line)" />
      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">{children}</div>
    </>
  );

  if (onSubmit) {
    return (
      <form onSubmit={onSubmit} className="flex h-full flex-col">
        {chrome}
      </form>
    );
  }
  return <div className="flex h-full flex-col">{chrome}</div>;
}

export function adapterDefaultEndpoint(
  adapters: AdapterDescriptor[],
  adapterId: string,
): string {
  return adapters.find((a) => a.id === adapterId)?.default_endpoint?.trim() ?? "";
}
