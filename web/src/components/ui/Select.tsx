import { type ReactNode } from "react";

import { Dropdown, dropdownItemClass, dropdownItemActiveClass } from "./Dropdown";

interface Option {
  value: string;
  label: ReactNode;
}

interface SelectProps {
  value: string;
  onChange: (value: string) => void;
  options: Option[];
  placeholder?: string;
  disabled?: boolean;
  direction?: "up" | "down";
  className?: string;
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="shrink-0 text-(--_dk-text-disabled) transition-transform duration-150"
      style={{ transform: open ? "rotate(180deg)" : "rotate(0deg)" }}
    >
      <path d="M3 1.5l4 3.5-4 3.5" />
    </svg>
  );
}

export function Select({
  value,
  onChange,
  options,
  placeholder,
  disabled,
  direction = "down",
  className = "",
}: SelectProps) {
  const selected = options.find((o) => o.value === value);
  const display = selected?.label ?? placeholder ?? "";

  return (
    <Dropdown
      direction={direction}
      variant="select"
      className={className}
      bgClassName="bg-(--_dk-overlay)"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          disabled={disabled}
          onClick={() => !disabled && toggle()}
          className="flex w-full items-center justify-between gap-1 border-0 border-b border-(--_dk-line) bg-transparent px-1 py-[0.375rem] text-left text-[0.875rem] text-(--_dk-text-muted) hover:brightness-110 focus-visible:border-(--_dk-line-visible) disabled:opacity-40"
        >
          <span className={selected ? "text-(--_dk-text-primary)" : "text-(--_dk-text-disabled)"}>
            {display}
          </span>
          <Chevron open={open} />
        </button>
      )}
    >
      {options.map((opt) => (
        <button
          key={opt.value}
          type="button"
          className={`${dropdownItemClass} ${opt.value === value ? dropdownItemActiveClass : ""}`}
          onClick={() => onChange(opt.value)}
        >
          {opt.label}
        </button>
      ))}
    </Dropdown>
  );
}
