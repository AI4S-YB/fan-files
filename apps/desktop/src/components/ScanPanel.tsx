import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// T17 事件流模式：invoke("scan_now") 只负责触发（立即返回），扫描进度/结束
// 由后端通过 scan://progress / scan://done / scan://error 事件推送。
// 挂载时轮询一次 scan_state 同步互斥标志（如托盘菜单已发起扫描）。
export default function ScanPanel({ onDone }: { onDone?: () => void }) {
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);

  useEffect(() => {
    let cancelled = false;
    invoke<boolean>("scan_state").then((s) => {
      if (!cancelled) setRunning(s);
    });
    const un1 = listen<string>("scan://progress", (e) =>
      setLines((ls) => [...ls, e.payload].slice(-500))
    );
    const un2 = listen<number>("scan://done", (e) => {
      setRunning(false);
      if (e.payload === 0 && onDone) onDone();
    });
    const un3 = listen<string>("scan://error", (e) => {
      // spawn 失败时后端复位互斥并推 error 事件，这里必须同步复位 running，
      // 否则按钮永远停在"扫描中…"禁用态
      setRunning(false);
      setLines((ls) => [...ls, `扫描失败: ${e.payload}`]);
    });
    return () => {
      cancelled = true;
      un1.then((u) => u());
      un2.then((u) => u());
      un3.then((u) => u());
    };
  }, [onDone]);

  async function scan() {
    setRunning(true);
    setLines([]);
    try {
      await invoke("scan_now");
    } catch (e) {
      setLines([`扫描失败: ${String(e)}`]);
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
