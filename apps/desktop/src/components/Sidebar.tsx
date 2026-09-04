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
        padding: "14px 10px",
        gap: 4,
        background: "hsl(var(--bg-elevated))",
        borderRight: "1px solid hsl(var(--border))",
        fontFamily: "var(--font-sans)",
        flexShrink: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 10px 12px",
          marginBottom: 8,
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
              className="transition-base"
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
                boxShadow: active ? "0 0 0 1px hsl(var(--primary) / .4)" : "none",
              }}
              onMouseEnter={(e) => {
                if (!active) {
                  e.currentTarget.style.background = "hsl(var(--bg))";
                  e.currentTarget.style.color = "hsl(var(--fg))";
                  e.currentTarget.style.transform = "translateX(2px)";
                }
              }}
              onMouseLeave={(e) => {
                if (!active) {
                  e.currentTarget.style.background = "transparent";
                  e.currentTarget.style.color = "hsl(var(--fg-muted))";
                  e.currentTarget.style.transform = "translateX(0)";
                }
              }}
            >
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