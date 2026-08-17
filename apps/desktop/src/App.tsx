import { useState } from "react";
import { Sidebar, Page } from "./components/Sidebar";
import HomePage from "./pages/HomePage";
import DatasetsPage from "./pages/DatasetsPage";
import SearchPage from "./pages/SearchPage";
import SettingsPage from "./pages/SettingsPage";
import "./App.css";

export default function App() {
  const [page, setPage] = useState<Page>("home");
  return (
    <div className="app">
      <Sidebar page={page} onSelect={setPage} />
      <main className="content">
        {page === "home" && <HomePage />}
        {page === "datasets" && <DatasetsPage />}
        {page === "search" && <SearchPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
