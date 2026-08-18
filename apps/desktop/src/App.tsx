import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sidebar, Page } from "./components/Sidebar";
import EngineBanner from "./components/EngineBanner";
import HomePage from "./pages/HomePage";
import DatasetsPage from "./pages/DatasetsPage";
import SearchPage from "./pages/SearchPage";
import SettingsPage from "./pages/SettingsPage";
import { setApiBase } from "./api";
import "./App.css";

export default function App() {
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
    <div className="app">
      <Sidebar page={page} onSelect={setPage} />
      <main className="content">
        <EngineBanner error={engineError} onRetry={retryEngine} />
        {page === "home" && <HomePage onGoSettings={() => setPage("settings")} />}
        {page === "datasets" && <DatasetsPage />}
        {page === "search" && <SearchPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
