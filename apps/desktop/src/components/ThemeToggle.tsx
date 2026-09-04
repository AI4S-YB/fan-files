import { useTheme } from "../hooks/useTheme";

const options: { value: "light" | "dark" | "system"; icon: string; label: string }[] = [
  { value: "light", icon: "☀️", label: "浅色" },
  { value: "dark",  icon: "🌙", label: "暗色" },
  { value: "system", icon: "💻", label: "跟随系统" },
];

export default function ThemeToggle({ compact = false }: { compact?: boolean }) {
  const [theme, setTheme] = useTheme();
  return (
    <div
      role="group"
      aria-label="主题切换"
      style={{
        display: "inline-flex",
        padding: 2,
        borderRadius: "var(--radius)",
        background: "hsl(var(--bg-elevated))",
        border: "1px solid hsl(var(--border))",
        gap: 2,
      }}
    >
      {options.map((o) => {
        const active = theme === o.value;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={active}
            title={o.label}
            onClick={() => setTheme(o.value)}
            style={{
              padding: compact ? "2px 6px" : "4px 10px",
              fontSize: compact ? 12 : 13,
              border: "none",
              borderRadius: "var(--radius-sm)",
              background: active ? "hsl(var(--bg))" : "transparent",
              color: active ? "hsl(var(--fg))" : "hsl(var(--fg-muted))",
              boxShadow: active
                ? "var(--shadow-sm), 0 0 0 1px hsl(var(--primary) / .5)"
                : "none",
              cursor: "pointer",
              transition: "all 150ms ease",
            }}
          >
            <span aria-hidden="true">{o.icon}</span>
            {!compact && <span style={{ marginLeft: 4 }}>{o.label}</span>}
          </button>
        );
      })}
    </div>
  );
}
