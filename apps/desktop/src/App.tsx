import { useState } from "react";
import { Sidebar, Page } from "./components/Sidebar";
import EngineBanner from "./components/EngineBanner";
import HomePage from "./pages/HomePage";
import DatasetsPage from "./pages/DatasetsPage";
import SearchPage from "./pages/SearchPage";
import SettingsPage from "./pages/SettingsPage";
import "./App.css";

export default function App() {
  const [page, setPage] = useState<Page>("home");
  const [engineError, setEngineError] = useState<string | null>(null);
  return (
    <div className="app">
      <Sidebar page={page} onSelect={setPage} />
      <main className="content">
        <EngineBanner error={engineError} onRetry={() => setEngineError(null)} />
        {page === "home" && <HomePage />}
        {page === "datasets" && <DatasetsPage />}
        {page === "search" && <SearchPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
