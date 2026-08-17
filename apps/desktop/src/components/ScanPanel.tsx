import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function ScanPanel() {
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);

  async function scan() {
    setRunning(true);
    setLines([]);
    try {
      // Task 17 注册 scan_now 后端命令并推送进度事件；当前会 reject
      await invoke("scan_now");
    } catch {
      setLines(["扫描命令尚未接入（Task 17）"]);
    } finally {
      setRunning(false);
    }
  }

  return (
    <div className="scan-panel">
      <button className="primary" disabled={running} onClick={scan}>
        {running ? "扫描中…" : "🔄 重新扫描"}
      </button>
      {lines.length > 0 && <pre className="scan-log">{lines.join("\n")}</pre>}
    </div>
  );
}
