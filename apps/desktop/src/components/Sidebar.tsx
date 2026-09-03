export type Page = "home" | "datasets" | "search" | "settings";

const items: { key: Page; icon: string; label: string }[] = [
  { key: "home", icon: "🏠", label: "首页" },
  { key: "datasets", icon: "📁", label: "数据集" },
  { key: "search", icon: "🔍", label: "搜索" },
  { key: "settings", icon: "⚙️", label: "设置" },
];

export function Sidebar({
  page,
  onSelect,
}: {
  page: Page;
  onSelect: (p: Page) => void;
}) {
  return (
    <nav className="sidebar">
      <div className="sidebar-logo">🌱 fan-files</div>
      {items.map((it) => (
        <button
          key={it.key}
          className={page === it.key ? "side-item active" : "side-item"}
          onClick={() => onSelect(it.key)}
        >
          <span>{it.icon}</span> {it.label}
        </button>
      ))}
      <div className="sidebar-version">early preview</div>
    </nav>
  );
}
