import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { fetchStats } from "../api";
import { useToast } from "./Toast";

// read_config 返回形状（与 SettingsPage 的 FanConfig 同构）；预检只看 api_key
interface FanConfig {
  api_key: string;
  [k: string]: unknown;
}

// T17 事件流模式：invoke("scan_now") 只负责触发（立即返回），扫描进度/结束
// 由后端通过 scan://progress / scan://done / scan://error 事件推送。
// 挂载时轮询一次 scan_state 同步互斥标志（如托盘菜单已发起扫描）。
// SF-T2：扫描前 LLM 预检（未配置 API Key 时 confirm 询问，跳过 Phase C 只做基础索引）；
// 完成/失败/错误均弹 toast 通知。
export default function ScanPanel({ onDone }: { onDone?: () => void }) {
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const { showToast } = useToast();

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
      if (e.payload === 0) {
        if (onDone) onDone();
        // 成功后拉一次统计拼"发现 N 个数据集"；失败（引擎未起等）退化为不带数的基础文案
        fetchStats()
          .then((stats) =>
            showToast(
              `扫描完成：发现 ${stats.datasets_upper_bound.toLocaleString()} 个数据集`,
              "success"
            )
          )
          .catch(() => showToast("扫描完成", "success"));
      } else {
        // 非 0 退出码：扫描未完成，日志里有阶段明细
        showToast("扫描失败，详见日志", "error");
      }
    });
    const un3 = listen<string>("scan://error", (e) => {
      // spawn 失败时后端复位互斥并推 error 事件，这里必须同步复位 running，
      // 否则按钮永远停在"扫描中…"禁用态
      setRunning(false);
      setLines((ls) => [...ls, `扫描失败: ${e.payload}`]);
      showToast("扫描失败，详见日志", "error");
    });
    return () => {
      cancelled = true;
      un1.then((u) => u());
      un2.then((u) => u());
      un3.then((u) => u());
    };
  }, [onDone, showToast]);

  // LLM 预检：read_config 失败按未配置处理（询问一次，确认后仍可继续基础索引）。
  // 取消 → 不发扫描；用户可去设置页配置模型。
  async function scan() {
    let cfg: FanConfig | null = null;
    try {
      cfg = await invoke<FanConfig>("read_config");
    } catch {
      /* 读取失败 → 视为未配置 */
    }
    if (!cfg?.api_key) {
      const ok = window.confirm(
        "未配置 LLM 模型，扫描将跳过数据集智能分类（Phase C），只做基础索引。是否继续？"
      );
      if (!ok) return;
    }
    setRunning(true);
    setLines([]);
    try {
      await invoke("scan_now");
    } catch (e) {
      setLines([`扫描失败: ${String(e)}`]);
      setRunning(false);
      showToast("扫描失败，详见日志", "error");
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
