import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Home, FolderOpen, Search, Settings, ChevronLeft, ChevronRight } from "lucide-react";
import { TitleBar } from "./components/TitleBar";
import EngineBanner from "./components/EngineBanner";
import HomePage from "./pages/HomePage";
import DatasetsPage from "./pages/DatasetsPage";
import SearchPage, { type ChatTurn } from "./pages/SearchPage";
import SettingsPage from "./pages/SettingsPage";
import ToastProvider from "./components/Toast";
import { setApiBase } from "./api";
import { useTheme } from "./hooks/useTheme";

type View = "home" | "datasets" | "search" | "settings";

const NAV = [
  { key: "home",     label: "首页",   icon: Home,        badge: undefined },
  { key: "datasets", label: "数据集", icon: FolderOpen,  badge: undefined },
  { key: "search",   label: "搜索",   icon: Search,      badge: "测试" },
  { key: "settings", label: "设置",   icon: Settings,    badge: undefined },
] as const;

const TITLES: Record<View, string> = {
  home: "首页",
  datasets: "数据集",
  search: "搜索",
  settings: "设置",
};

export default function App() {
  useTheme();
  const [view, setView] = useState<View>("home");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [engineError, setEngineError] = useState<string | null>(null);
  const [updateText, setUpdateText] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  // 会话状态提升到 App 层，这样切走再切回搜索页不会丢失对话历史
  const [turns, setTurns] = useState<ChatTurn[]>([]);

  // 挂载时：拿 share 实际端口设置 API base；读一次引擎错误并每 5 秒轮询同步。
  useEffect(() => {
    let cancelled = false;
    invoke<number>("get_share_port")
      .then(setApiBase)
      .catch(() => {});
    const sync = async () => {
      try {
        const err = await invoke<string | null>("engine_error");
        if (!cancelled) setEngineError(err);
      } catch (e) {
        if (!cancelled) setEngineError(String(e));
      }
    };
    sync();
    const timer = setInterval(sync, 5000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  // 扫描完成（scan://done payload=0）→ 广播 fan-scan-done 全局事件。
  useEffect(() => {
    const un = listen<number>("scan://done", (e) => {
      if (e.payload === 0) window.dispatchEvent(new CustomEvent("fan-scan-done"));
    });
    return () => {
      un.then((u) => u());
    };
  }, []);

  const retryEngine = useCallback(async () => {
    try {
      const port = await invoke<number>("retry_engine");
      setApiBase(port);
      setEngineError(null);
    } catch (e) {
      setEngineError(String(e));
    }
  }, []);

  const checkUpdate = useCallback(async () => {
    setCheckingUpdate(true);
    setUpdateText(null);
    try {
      const text = await invoke<string>("check_update");
      setUpdateText(text);
    } catch (e) {
      setUpdateText(String(e));
    } finally {
      setCheckingUpdate(false);
    }
  }, []);

  return (
    <ToastProvider>
      <div className="app">
        <TitleBar
          version="v0.2.6"
          updateMsg={updateText}
          checkingUpdate={checkingUpdate}
          onCheckUpdate={checkUpdate}
        />
        <div className="layout">
          {/* Sidebar with text labels (primary navigation) */}
          <nav className="sidebar" aria-label="主导航" data-collapsed={sidebarCollapsed}>
            <div className="sidebar-nav">
              {NAV.map(({ key, label, icon: Icon, badge }) => {
                const active = view === key;
                return (
                  <button
                    key={key}
                    type="button"
                    aria-current={active ? "page" : undefined}
                    onClick={() => setView(key)}
                    className={`sidebar-btn ${active ? "active" : ""}`}
                  >
                    <Icon size={16} />
                    <span className="sidebar-btn-label">{label}</span>
                    {badge && <span className="sidebar-badge">{badge}</span>}
                  </button>
                );
              })}
            </div>
            <button
              className="sidebar-collapse-btn"
              onClick={() => setSidebarCollapsed((c) => !c)}
              aria-label={sidebarCollapsed ? "展开菜单" : "收起菜单"}
              title={sidebarCollapsed ? "展开菜单" : "收起菜单"}
            >
              {sidebarCollapsed ? <ChevronRight size={13} /> : <ChevronLeft size={13} />}
            </button>
          </nav>

          {/* Main content */}
          <main className="main">
            <header className="main-header">
              <h1>{TITLES[view]}</h1>
            </header>
            <div className="main-body">
              <EngineBanner error={engineError} onRetry={retryEngine} />
              {view === "home" && (
                <HomePage onGoSettings={() => setView("settings")} />
              )}
              {view === "datasets" && <DatasetsPage />}
              {view === "search" && <SearchPage turns={turns} setTurns={setTurns} />}
              {view === "settings" && <SettingsPage />}
            </div>
          </main>
        </div>
      </div>
    </ToastProvider>
  );
}