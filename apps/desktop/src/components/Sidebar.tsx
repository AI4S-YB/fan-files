import { Home, FolderOpen, Search, Settings } from "lucide-react";
import ThemeToggle from "./ThemeToggle";

export type Page = "home" | "datasets" | "search" | "settings";

const items: { key: Page; icon: typeof Home; label: string }[] = [
  { key: "home",     icon: Home,       label: "首页" },
  { key: "datasets", icon: FolderOpen, label: "数据集" },
  { key: "search",   icon: Search,     label: "搜索" },
  { key: "settings", icon: Settings,   label: "设置" },
];

export function Sidebar({
  page,
  onSelect,
}: {
  page: Page;
  onSelect: (p: Page) => void;
}) {
  return (
    <nav className="sidebar" aria-label="主导航">
      <div className="sidebar-nav">
        {items.map((item) => {
          const active = page === item.key;
          return (
            <button
              key={item.key}
              type="button"
              className={`sidebar-btn ${active ? "active" : ""}`}
              onClick={() => onSelect(item.key)}
            >
              <item.icon size={15} />
              <span className="sidebar-btn-label">{item.label}</span>
            </button>
          );
        })}
      </div>
      <div className="sidebar-bottom">
        <ThemeToggle compact />
      </div>
    </nav>
  );
}
