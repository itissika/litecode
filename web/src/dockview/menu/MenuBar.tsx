import type { CSSProperties } from "react";

import { buildMenuItems, type MenuItem } from "./menuItems";
import { Dropdown, dropdownItemClass } from "../../components/ui/Dropdown";

interface MenuBarProps {
  onAction?: (item: string) => void;
  sessionMode?: "local" | "remote";
  items?: MenuItem[];
}

export function MenuBar({ onAction, sessionMode = "local", items }: MenuBarProps) {
  const menus = items ?? buildMenuItems(sessionMode);
  return (
    <div
      className="flex items-center h-full"
      style={{ WebkitAppRegion: "no-drag" } as CSSProperties}
    >
      {menus.map((menu) => (
        <Dropdown
          key={menu.label}
          variant="menu"
          trigger={({ open, toggle }) => (
            <button
              className="px-2.5 h-[32px] text-xs hover:bg-(--_dk-ix-bg-hover) hover:brightness-125 active:brightness-75"
              style={{
                background: open ? "var(--_dk-pressed)" : undefined,
                color: open
                  ? "var(--_dk-text-primary)"
                  : "var(--_dk-text-secondary)",
              }}
              onClick={toggle}
            >
              {menu.label}
            </button>
          )}
        >
          {menu.items.map((item, i) =>
            item === "separator" ? (
              <div
                key={i}
                className="mx-2 my-1"
                style={{ borderTop: "1px solid var(--_dk-line)" }}
              />
            ) : (
              <button
                key={item}
                className={`${dropdownItemClass} text-(--_dk-text-secondary)`}
                onClick={() => onAction?.(item)}
              >
                {item}
              </button>
            ),
          )}
        </Dropdown>
      ))}
    </div>
  );
}
