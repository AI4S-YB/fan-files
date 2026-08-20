import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { fetchStats, type Stats } from "../api";
import ScanPanel from "../components/ScanPanel";

// 与后端 T5 read_config 命令返回的形状一致（见 crates 侧 FanConfig）。
interface FanConfig {
  include: string[];
  exclude: string[];
  endpoint: string;
  api_key: string;
  model: string;
}

// last_indexed_at 为 Unix 秒；显示 yyyy-mm-dd hh:mm 本地时间，null 显示 "—"。
function formatLastScan(ts: number | null): string {
  if (ts === null) return "—";
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export default function HomePage({ onGoSettings }: { onGoSettings: () => void }) {
  const [stats, setStats] = useState<Stats | null>(null);
  const [configured, setConfigured] = useState(false);
  const [loading, setLoading] = useState(true);

  // useCallback：身份稳定，避免每次重渲染都换 onDone 引用导致 ScanPanel
  // 反复退订/重订阅 scan:// 事件
  const refreshStats = useCallback(() => {
    fetchStats()
      .then(setStats)
      .catch((e) => {
        // 静默失败：不阻塞界面（保留旧统计为空态），仅打印便于排查
        console.error("fetchStats failed:", e);
        setStats(null);
      });
  }, []);

  useEffect(() => {
    invoke<FanConfig>("read_config")
      .then((cfg) => setConfigured(cfg.include.length > 0))
      .catch(() => setConfigured(false))
      .finally(() => setLoading(false));
    refreshStats();
  }, []);

  const lastScan = stats ? formatLastScan(stats.last_indexed_at) : "—";
  // GUI-T5 修复 [规格 §八]: Stats.approximate 为真时统计卡数字加 ~ 前缀
  //（增量索引统计为近似值；最近扫描时间不受影响）
  const approx = stats?.approximate ? "~" : "";
  const countTitle = stats?.approximate ? "近似值（增量索引统计）" : undefined;

  return (
    <div className="page">
      <h2>首页</h2>
      {loading ? null : !configured ? (
        <div className="empty-cta">
          <p>先告诉 fan-files 你的数据在哪里</p>
          <button className="primary" onClick={onGoSettings}>
            📁 选择目录开始扫描
          </button>
        </div>
      ) : (
        <>
          <div className="stat-cards">
            <div className="stat-card" title={countTitle}>
              <b>{stats ? `${approx}${stats.datasets_upper_bound.toLocaleString()}` : "—"}</b>
              <span>数据集</span>
            </div>
            {/* GUI-T4: 统计卡补全资产数（Stats.assets_upper_bound 由后端聚合） */}
            <div className="stat-card" title={countTitle}>
              <b>{stats ? `${approx}${stats.assets_upper_bound.toLocaleString()}` : "—"}</b>
              <span>资产</span>
            </div>
            <div className="stat-card" title={countTitle}>
              <b>{stats ? `${approx}${stats.files_upper_bound.toLocaleString()}` : "—"}</b>
              <span>文件</span>
            </div>
            <div className="stat-card">
              <b>{lastScan}</b>
              <span>最近扫描</span>
            </div>
          </div>
          <ScanPanel onDone={refreshStats} />
        </>
      )}
    </div>
  );
}
