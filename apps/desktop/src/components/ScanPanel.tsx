import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ScanPanelProps {
  // 扫描成功后回调，用于刷新统计卡（Task 17 接入 scan_now 后生效）。
  onDone?: () => void;
}

export default function ScanPanel({ onDone }: ScanPanelProps) {
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);

  async function scan() {
    setRunning(true);
    setLines([]);
    try {
      // Task 17 注册 scan_now 后端命令并推送进度事件；当前会 reject
      await invoke("scan_now");
      onDone?.();
    } catch (e) {
      setLines([`扫描失败: ${String(e)}`]);
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
