import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Download, FolderOpen, HardDrive, Files, Clock, ScanLine, Share2, ArrowUpRight, ArrowDownLeft, Activity, AlertCircle } from "lucide-react";
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

// transfer_history Tauri 命令返回形状（见 crates 侧 HistoryEntry）。
interface TransferEntry {
  direction: string;          // "send" | "receive"
  dataset: string;
  code: string;
  status: string;             // "ok" | "err" | "running" | ...
  bytes_sent: number;
  bytes_received: number;
  time: number;               // Unix 秒
  code_used_at: number | null;
  completed_at: number | null;
}

// 当前进行中的传输：监听 share://receive://progress 事件累积 + 状态机
interface ActiveTransfer {
  id: string;                 // share dataset path or code
  direction: "send" | "receive";
  dataset: string;
  code?: string;
  status: "running" | "code" | "ok" | "err";
  bytesSent: number;
  bytesTotal: number;
  startedAt: number;
}

// last_indexed_at 为 Unix 秒；显示 yyyy-mm-dd hh:mm 本地时间，null 显示 "—"。
function formatLastScan(ts: number | null): string {
  if (ts === null) return "尚未扫描";
  const d = new Date(ts * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function timeAgo(ts: number): string {
  const sec = Math.floor(Date.now() / 1000) - ts;
  if (sec < 60) return `${sec} 秒前`;
  if (sec < 3600) return `${Math.floor(sec / 60)} 分钟前`;
  if (sec < 86400) return `${Math.floor(sec / 3600)} 小时前`;
  return `${Math.floor(sec / 86400)} 天前`;
}

function statusLabel(h: TransferEntry): { text: string; color: "ok" | "err" | "warn" | "muted" } {
  if (h.status !== "ok") return { text: "失败", color: "err" };
  if (h.completed_at) return { text: "已完成", color: "ok" };
  if (h.code_used_at) return { text: "已被使用", color: "ok" };
  return { text: "已发码", color: "warn" };
}

export default function HomePage({ onGoSettings }: { onGoSettings: () => void }) {
  const [stats, setStats] = useState<Stats | null>(null);
  const [configured, setConfigured] = useState(false);
  const [loading, setLoading] = useState(true);
  const [history, setHistory] = useState<TransferEntry[]>([]);
  const [activeTransfers, setActiveTransfers] = useState<ActiveTransfer[]>([]);

  const refreshStats = useCallback(() => {
    fetchStats()
      .then(setStats)
      .catch((e) => {
        console.error("fetchStats failed:", e);
        setStats(null);
      });
  }, []);

  const refreshHistory = useCallback(() => {
    invoke<TransferEntry[]>("transfer_history")
      .then((h) => setHistory(h ?? []))
      .catch(() => setHistory([]));
  }, []);

  useEffect(() => {
    invoke<FanConfig>("read_config")
      .then((cfg) => setConfigured(cfg.include.length > 0))
      .catch(() => setConfigured(false))
      .finally(() => setLoading(false));
    refreshStats();
    refreshHistory();

    // 监听扫描完成 → 刷新统计
    const onScanDone = () => { refreshStats(); };
    window.addEventListener("fan-scan-done", onScanDone);

    // 监听 share://progress 累积进行中的传输
    const unShare = listen<string>("share://progress", (e) => {
      try {
        const ev = JSON.parse(e.payload);
        if (ev.type === "code") {
          setActiveTransfers((s) => [
            ...s.filter((t) => t.id !== `share:${ev.code}`),
            {
              id: `share:${ev.code}`,
              direction: "send",
              dataset: ev.dataset ?? "—",
              code: ev.code,
              status: "code",
              bytesSent: 0,
              bytesTotal: 0,
              startedAt: Math.floor(Date.now() / 1000),
            },
          ]);
        } else if (ev.type === "progress") {
          setActiveTransfers((s) =>
            s.map((t) =>
              t.id.startsWith("share:") && t.status === "code"
                ? { ...t, status: "running", bytesSent: ev.sent, bytesTotal: ev.total }
                : t
            )
          );
        }
      } catch { /* 忽略非 JSON 行 */ }
    });

    // 监听 receive://progress
    const unRecv = listen<string>("receive://progress", (e) => {
      try {
        const ev = JSON.parse(e.payload);
        if (ev.type === "progress") {
          setActiveTransfers((s) => [
            ...s.filter((t) => !t.id.startsWith("recv:")),
            {
              id: `recv:${ev.code ?? Math.random()}`,
              direction: "receive",
              dataset: ev.dataset ?? "—",
              code: ev.code,
              status: "running",
              bytesSent: ev.sent,
              bytesTotal: ev.total,
              startedAt: Math.floor(Date.now() / 1000),
            },
          ]);
        }
      } catch { /* ignore */ }
    });

    // 监听 share://done 与 receive://done
    const unShareDone = listen<number>("share://done", () => {
      setActiveTransfers((s) => s.filter((t) => !t.id.startsWith("share:")));
      refreshHistory();
    });
    const unRecvDone = listen<number>("receive://done", () => {
      setActiveTransfers((s) => s.filter((t) => !t.id.startsWith("recv:")));
      refreshHistory();
    });

    return () => {
      window.removeEventListener("fan-scan-done", onScanDone);
      unShare.then((u) => u());
      unRecv.then((u) => u());
      unShareDone.then((u) => u());
      unRecvDone.then((u) => u());
    };
  }, [refreshStats, refreshHistory]);

  const approx = stats?.approximate ? "~" : "";
  const countTitle = stats?.approximate ? "近似值（增量索引统计）" : undefined;

  return (
    <div className="home-page">
      {loading ? null : !configured ? (
        <div className="card" style={{ maxWidth: 520, margin: "40px auto", textAlign: "center" }}>
          <div className="card-body" style={{ padding: "48px 40px" }}>
            <div
              style={{
                width: 64,
                height: 64,
                margin: "0 auto 20px",
                borderRadius: 16,
                background: "hsl(var(--primary) / .12)",
                display: "grid",
                placeItems: "center",
              }}
            >
              <FolderOpen size={30} style={{ color: "hsl(var(--primary))" }} />
            </div>
            <h2 style={{ margin: "0 0 8px", fontSize: 18, fontWeight: 700 }}>先告诉 fan-files 你的数据在哪里</h2>
            <p className="muted" style={{ margin: "0 0 24px", fontSize: 13, lineHeight: 1.6 }}>
              添加数据目录后，fan-files 会扫描其中的数据集并建立索引，
              之后即可在本机检索、分享和同步。
            </p>
            <button className="btn btn-primary" onClick={onGoSettings} style={{ height: 38, padding: "0 24px", fontSize: 14 }}>
              选择目录开始
            </button>
          </div>
        </div>
      ) : (
        <>
          {/* 统计卡片 */}
          <div className="stat-grid">
            {[
              { label: "数据集", value: stats?.datasets_upper_bound, icon: HardDrive, hint: "已索引的数据集" },
              { label: "资产", value: stats?.assets_upper_bound, icon: Files, hint: "数据资产总数" },
              { label: "文件", value: stats?.files_upper_bound, icon: Download, hint: "底层文件数" },
            ].map(({ label, value, icon: Icon, hint }) => (
              <div key={label} className="card hover-lift" style={{ position: "relative", overflow: "hidden" }}>
                <div
                  style={{
                    position: "absolute",
                    top: 0, left: 0, right: 0, height: 3,
                    background: "linear-gradient(90deg, hsl(var(--primary)), hsl(var(--accent-2)))",
                  }}
                />
                <div className="card-body" style={{ padding: "20px" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 14 }}>
                    <span
                      style={{
                        width: 34, height: 34, borderRadius: 9,
                        background: "hsl(var(--primary) / .1)",
                        display: "grid", placeItems: "center",
                      }}
                    >
                      <Icon size={17} style={{ color: "hsl(var(--primary))" }} />
                    </span>
                    <span style={{ fontSize: 12, fontWeight: 600, color: "hsl(var(--text-dim))" }}>{label}</span>
                  </div>
                  <div
                    style={{
                      fontSize: 28,
                      fontWeight: 700,
                      color: "hsl(var(--text))",
                      fontVariantNumeric: "tabular-nums",
                      letterSpacing: "-0.02em",
                    }}
                    title={countTitle}
                  >
                    {value != null ? `${approx}${value.toLocaleString()}` : "—"}
                  </div>
                  <div style={{ fontSize: 11, color: "hsl(var(--text-faint))", marginTop: 6 }}>{hint}</div>
                </div>
              </div>
            ))}
            <div className="card hover-lift" style={{ position: "relative", overflow: "hidden" }}>
              <div
                style={{
                  position: "absolute", top: 0, left: 0, right: 0, height: 3,
                  background: "linear-gradient(90deg, hsl(var(--success)), hsl(var(--accent-2)))",
                }}
              />
              <div className="card-body" style={{ padding: "20px" }}>
                <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 14 }}>
                  <span
                    style={{
                      width: 34, height: 34, borderRadius: 9,
                      background: "hsl(var(--success) / .12)",
                      display: "grid", placeItems: "center",
                    }}
                  >
                    <Clock size={17} style={{ color: "hsl(var(--success))" }} />
                  </span>
                  <span style={{ fontSize: 12, fontWeight: 600, color: "hsl(var(--text-dim))" }}>最近扫描</span>
                </div>
                <div
                  style={{
                    fontSize: 15,
                    fontWeight: 600,
                    color: "hsl(var(--text))",
                    fontVariantNumeric: "tabular-nums",
                    letterSpacing: "-0.01em",
                  }}
                >
                  {stats ? formatLastScan(stats.last_indexed_at) : "尚未扫描"}
                </div>
                <div style={{ fontSize: 11, color: "hsl(var(--text-faint))", marginTop: 6 }}>上次索引时间</div>
              </div>
            </div>
          </div>

          {/* 传输状态卡片 */}
          <section className="card" style={{ overflow: "hidden" }}>
            <div className="card-header">
              <h2><Activity size={14} style={{ verticalAlign: -2, marginRight: 6 }} />传输状态</h2>
              {activeTransfers.length > 0 && (
                <span className="badge badge-info">{activeTransfers.length} 进行中</span>
              )}
            </div>
            <div className="card-body" style={{ padding: 0 }}>
              {/* 进行中的传输 */}
              {activeTransfers.length > 0 && (
                <div className="active-transfers">
                  {activeTransfers.map((t) => {
                    const pct = t.bytesTotal > 0 ? Math.round((t.bytesSent / t.bytesTotal) * 100) : 0;
                    return (
                      <div key={t.id} className="active-transfer-row">
                        <div className="active-transfer-icon">
                          {t.direction === "send" ? (
                            <ArrowUpRight size={14} style={{ color: "hsl(var(--primary))" }} />
                          ) : (
                            <ArrowDownLeft size={14} style={{ color: "hsl(var(--accent-2))" }} />
                          )}
                        </div>
                        <div className="active-transfer-info">
                          <div className="active-transfer-title">
                            {t.direction === "send" ? "分享中" : "接收中"}：{t.dataset}
                            {t.code && <span className="muted"> · {t.code}</span>}
                          </div>
                          <div className="active-transfer-meta">
                            {t.status === "code" ? (
                              <span>等待对方使用配对码…</span>
                            ) : t.bytesTotal > 0 ? (
                              <span>{formatBytes(t.bytesSent)} / {formatBytes(t.bytesTotal)} · {pct}%</span>
                            ) : (
                              <span>连接中…</span>
                            )}
                          </div>
                          {t.status === "running" && t.bytesTotal > 0 && (
                            <div className="active-transfer-bar">
                              <div className="active-transfer-bar-fill" style={{ width: `${pct}%` }} />
                            </div>
                          )}
                        </div>
                        <span className="spinner" />
                      </div>
                    );
                  })}
                </div>
              )}

              {/* 历史共享数据集 */}
              <div className="home-history">
                <div className="home-history-header">
                  <Share2 size={12} />
                  <span>最近共享 / 接收</span>
                </div>
                {history.length === 0 ? (
                  <div className="home-history-empty">
                    <AlertCircle size={14} />
                    <span>暂无历史记录</span>
                  </div>
                ) : (
                  <ul className="home-history-list">
                    {history.slice(0, 5).map((h, i) => {
                      const st = statusLabel(h);
                      return (
                        <li key={i} className="home-history-row">
                          <div className="home-history-icon">
                            {h.direction === "send" ? (
                              <ArrowUpRight size={13} />
                            ) : (
                              <ArrowDownLeft size={13} />
                            )}
                          </div>
                          <div className="home-history-info">
                            <div className="home-history-title">{h.dataset}</div>
                            <div className="home-history-meta">
                              <span>{h.direction === "send" ? "分享" : "接收"}</span>
                              <span className="home-history-dot">·</span>
                              <span>{h.code}</span>
                              <span className="home-history-dot">·</span>
                              <span>{timeAgo(h.time)}</span>
                            </div>
                          </div>
                          <div className="home-history-right">
                            <span className={`badge badge-${st.color}`}>{st.text}</span>
                            <span className="home-history-bytes">
                              {formatBytes(h.bytes_sent + h.bytes_received)}
                            </span>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>
            </div>
          </section>

          {/* 扫描面板 */}
          <section className="card" style={{ marginTop: 8 }}>
            <div className="card-header">
              <h2><ScanLine size={14} style={{ verticalAlign: -2, marginRight: 6 }} />扫描索引</h2>
            </div>
            <div className="card-body">
              <ScanPanel onDone={refreshStats} />
            </div>
          </section>
        </>
      )}
    </div>
  );
}