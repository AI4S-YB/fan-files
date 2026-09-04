import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Sidebar, Page } from "./components/Sidebar";
import EngineBanner from "./components/EngineBanner";
import ThemeToggle from "./components/ThemeToggle";
import HomePage from "./pages/HomePage";
import DatasetsPage from "./pages/DatasetsPage";
import SearchPage from "./pages/SearchPage";
import SettingsPage from "./pages/SettingsPage";
import ToastProvider from "./components/Toast";
import { AppShell } from "./components/AppShell";
import { setApiBase } from "./api";
import { useTheme } from "./hooks/useTheme";
import "./App.css";

const TITLE: Record<Page, string> = {
  home: "首页",
  datasets: "数据集",
  search: "搜索",
  settings: "设置",
};

export default function App() {
  useTheme();
  const [page, setPage] = useState<Page>("home");
  const [engineError, setEngineError] = useState<string | null>(null);

  // 挂载时：拿 share 实际端口设置 API base；读一次引擎错误并每 5 秒轮询同步。
  useEffect(() => {
    let cancelled = false;
    invoke<number>("get_share_port")
      .then(setApiBase)
      .catch(() => {
        /* 保持默认端口 */
      });
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

  // SF-T3: 扫描完成（scan://done payload=0）→ 广播 fan-scan-done 全局事件。
  // 广播放 App 层：页面级订阅会随页面切换卸载（ScanPanel 在首页，扫描完成时
  // 用户可能已切到数据集/搜索页），App 常驻最稳。ScanPanel 自己的监听不受影响
  //（toast + 首页统计刷新照旧，两条独立订阅）。
  useEffect(() => {
    const un = listen<number>("scan://done", (e) => {
      if (e.payload === 0) window.dispatchEvent(new CustomEvent("fan-scan-done"));
    });
    return () => {
      un.then((u) => u());
    };
  }, []);

  const retryEngine = async () => {
    try {
      // 重试可能发生端口回退，retry_engine 返回实际端口——用它更新 API base，
      // 否则后续请求仍指向旧端口。
      const port = await invoke<number>("retry_engine");
      setApiBase(port);
      setEngineError(null);
    } catch (e) {
      setEngineError(String(e));
    }
  };

  return (
    <ToastProvider>
      <div className="app" style={{ display: "flex", height: "100vh" }}>
        <Sidebar page={page} onSelect={setPage} />
        <AppShell
          title={TITLE[page]}
          actions={<ThemeToggle compact />}
        >
          <EngineBanner error={engineError} onRetry={retryEngine} />
          {page === "home" && (
            <div key="home" className="anim-fade-up">
              <HomePage onGoSettings={() => setPage("settings")} />
            </div>
          )}
          {page === "datasets" && (
            <div key="datasets" className="anim-fade-up">
              <DatasetsPage />
            </div>
          )}
          {page === "search" && (
            <div key="search" className="anim-fade-up">
              <SearchPage />
            </div>
          )}
          {page === "settings" && (
            <div key="settings" className="anim-fade-up">
              <SettingsPage />
            </div>
          )}
        </AppShell>
      </div>
    </ToastProvider>
  );
}
