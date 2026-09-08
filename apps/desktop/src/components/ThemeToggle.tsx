import { Sun, Moon, Monitor } from "lucide-react";
import { useTheme } from "../hooks/useTheme";

const options: { value: "light" | "dark" | "system"; Icon: typeof Sun; label: string; shortcut: string }[] = [
  { value: "light",  Icon: Sun,     label: "浅色",  shortcut: "L" },
  { value: "dark",   Icon: Moon,    label: "暗色",  shortcut: "D" },
  { value: "system", Icon: Monitor, label: "跟随系统", shortcut: "S" },
];

export default function ThemeToggle({ compact = false }: { compact?: boolean }) {
  const [theme, setTheme] = useTheme();
  return (
    <div
      role="group"
      aria-label="主题切换"
      className={`theme-toggle ${compact ? "theme-toggle-compact" : ""}`}
    >
      {options.map((o) => {
        const active = theme === o.value;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={active}
            title={`${o.label} (⌘${o.shortcut})`}
            aria-keyshortcuts={`Meta+${o.shortcut}`}
            onClick={() => setTheme(o.value)}
            className={`theme-toggle-btn ${active ? "active" : ""}`}
          >
            <o.Icon size={compact ? 12 : 13} />
            {!compact && <span>{o.label}</span>}
          </button>
        );
      })}
    </div>
  );
}
