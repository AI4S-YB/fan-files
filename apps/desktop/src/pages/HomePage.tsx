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
      {loading ? null : !configured ? (
        <div
          className="empty-cta anim-fade-up"
          style={{
            padding: "60px 40px",
            textAlign: "center",
            background: "hsl(var(--bg-elevated))",
            border: "1px dashed hsl(var(--border))",
            borderRadius: "var(--radius-lg)",
            maxWidth: 480,
            margin: "40px auto",
          }}
        >
          <div style={{ fontSize: 40, marginBottom: 12 }}>📁</div>
          <p
            style={{
              color: "hsl(var(--fg))",
              fontSize: 15,
              fontWeight: 500,
              margin: "0 0 16px",
            }}
          >
            先告诉 fan-files 你的数据在哪里
          </p>
          <button
            type="button"
            onClick={onGoSettings}
            className="btn btn-primary transition-base"
            style={{ height: 38, padding: "0 20px", fontSize: 14 }}
          >
            选择目录开始扫描
          </button>
        </div>
      ) : (
        <>
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(160px, 1fr))",
              gap: 12,
              marginBottom: 20,
            }}
          >
            {[
              { label: "数据集", value: stats?.datasets_upper_bound, icon: "🗂️" },
              { label: "资产", value: stats?.assets_upper_bound, icon: "📦" },
              { label: "文件", value: stats?.files_upper_bound, icon: "📄" },
              { label: "最近扫描", value: null, icon: "🕐", text: lastScan },
            ].map((c) => (
              <div
                key={c.label}
                className="card-interactive"
                style={{
                  position: "relative",
                  padding: 16,
                  overflow: "hidden",
                }}
              >
                <div
                  aria-hidden="true"
                  style={{
                    position: "absolute",
                    top: 0, left: 0, right: 0,
                    height: 3,
                    background:
                      "linear-gradient(90deg, hsl(var(--primary)), hsl(199 89% 60%))",
                  }}
                />
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    fontSize: 12,
                    color: "hsl(var(--fg-muted))",
                    marginBottom: 8,
                  }}
                >
                  <span
                    aria-hidden="true"
                    style={{
                      width: 22,
                      height: 22,
                      borderRadius: 6,
                      background: "hsl(var(--bg))",
                      border: "1px solid hsl(var(--border))",
                      display: "grid",
                      placeItems: "center",
                      fontSize: 12,
                    }}
                  >
                    {c.icon}
                  </span>
                  <span>{c.label}</span>
                </div>
                <div
                  style={{
                    fontSize: 24,
                    fontWeight: 700,
                    color: "hsl(var(--fg))",
                    fontVariantNumeric: "tabular-nums",
                    letterSpacing: "-0.02em",
                  }}
                  title={countTitle}
                >
                  {c.text !== undefined
                    ? c.text
                    : c.value != null
                      ? `${approx}${c.value.toLocaleString()}`
                      : "—"}
                </div>
              </div>
            ))}
          </div>
          <ScanPanel onDone={refreshStats} />
        </>
      )}
    </div>
  );
}
