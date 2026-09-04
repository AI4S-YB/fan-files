import ThemeToggle from "./ThemeToggle";

export type Page = "home" | "datasets" | "search" | "settings";

const items: { key: Page; icon: string; label: string }[] = [
  { key: "home",     icon: "🏠", label: "首页" },
  { key: "datasets", icon: "📁", label: "数据集" },
  { key: "search",   icon: "🔍", label: "搜索" },
  { key: "settings", icon: "⚙️", label: "设置" },
];

const SIDEBAR_WIDTH = 200;

export function Sidebar({
  page,
  onSelect,
}: {
  page: Page;
  onSelect: (p: Page) => void;
}) {
  return (
    <nav
      aria-label="主导航"
      style={{
        width: SIDEBAR_WIDTH,
        display: "flex",
        flexDirection: "column",
        padding: "12px 8px",
        gap: 4,
        background: "hsl(var(--bg-elevated))",
        borderRight: "1px solid hsl(var(--border))",
        fontFamily: "var(--font-sans)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 10px 12px",
          marginBottom: 4,
          borderBottom: "1px solid hsl(var(--border))",
          fontSize: 14,
          fontWeight: 700,
          color: "hsl(var(--fg))",
          letterSpacing: "-0.01em",
        }}
      >
        <span style={{ fontSize: 18 }}>🌱</span>
        <span>fan-files</span>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 2, marginTop: 4 }}>
        {items.map((it) => {
          const active = page === it.key;
          return (
            <button
              key={it.key}
              type="button"
              aria-current={active ? "page" : undefined}
              onClick={() => onSelect(it.key)}
              style={{
                position: "relative",
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "8px 10px",
                fontFamily: "inherit",
                fontSize: 13,
                fontWeight: active ? 600 : 500,
                color: active ? "hsl(var(--fg))" : "hsl(var(--fg-muted))",
                background: active ? "hsl(var(--primary) / .08)" : "transparent",
                border: "none",
                borderRadius: "var(--radius)",
                cursor: "pointer",
                textAlign: "left",
                transition: "background 120ms ease, color 120ms ease",
              }}
              onMouseEnter={(e) => {
                if (!active) e.currentTarget.style.background = "hsl(var(--bg))";
              }}
              onMouseLeave={(e) => {
                if (!active) e.currentTarget.style.background = "transparent";
              }}
            >
              {active && (
                <span
                  aria-hidden="true"
                  style={{
                    position: "absolute",
                    left: 0,
                    top: 6,
                    bottom: 6,
                    width: 3,
                    borderRadius: 2,
                    background: "hsl(var(--primary))",
                  }}
                />
              )}
              <span aria-hidden="true" style={{ fontSize: 15, width: 18, textAlign: "center" }}>
                {it.icon}
              </span>
              <span>{it.label}</span>
            </button>
          );
        })}
      </div>

      <div style={{ flex: 1 }} />

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 8,
          padding: "8px 4px",
          borderTop: "1px solid hsl(var(--border))",
        }}
      >
        <ThemeToggle compact />
        <div
          style={{
            fontSize: 11,
            color: "hsl(var(--fg-subtle))",
            textAlign: "center",
            padding: "0 4px",
          }}
        >
          v0.2.6 · early preview
        </div>
      </div>
    </nav>
  );
}