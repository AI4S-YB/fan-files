import { RefreshCw } from "lucide-react";

export function TitleBar({
  version,
  updateMsg,
  checkingUpdate,
  onCheckUpdate,
}: {
  version: string;
  updateMsg: string | null;
  checkingUpdate: boolean;
  onCheckUpdate: () => void;
}) {
  return (
    <div className="titlebar">
      <div className="titlebar-brand">
        <span className="logo">
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <path d="M6 1L10.5 4V8L6 11L1.5 8V4L6 1Z" fill="white" />
          </svg>
        </span>
        <span className="sidebar-name">fan-files</span>
      </div>
      <div className="titlebar-status">
        <span className="status-dot" />
        <span>{version}</span>
        {updateMsg && !checkingUpdate && (
          <span className="update-pill">
            <RefreshCw size={10} />
            {updateMsg.includes("New") ? "检查更新" : "已是最新"}
          </span>
        )}
        <button className="lang-toggle" onClick={onCheckUpdate} title="检查更新" aria-label="检查更新">
          <RefreshCw size={12} />
        </button>
      </div>
    </div>
  );
}
