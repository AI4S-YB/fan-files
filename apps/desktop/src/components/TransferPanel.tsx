import React, { useEffect, useMemo, useRef } from "react";

// 传输事件（引擎 FAN_JSON_PROGRESS=1 的 JSONL 事件行 JSON.parse 后分发）。
// 字段与 crates/fan-files/src/commands/transfer.rs 的 json_event_value 一致。
export type TransferEvent =
  | { type: "progress"; sent: number; total: number; pct: number; chunks: number }
  | { type: "conn"; mode: "direct" | "relay" | "punching" }
  | { type: "resume"; done: number; total: number }
  | { type: "done"; ok: boolean; bytes: number; elapsed_secs: number }
  | { type: "error"; msg: string };

interface Props {
  name: string; // 文件名（接收侧为配对码）
  events: TransferEvent[]; // 已解析事件流（页面分发后传入）
  log: string[]; // 原始行（含非 JSON 的人类输出，失败原因可见）
  onCancel: () => void; // 取消当前传输
}

// 字节 → 人类可读（B/KB/MB/GB，保留 1 位小数）
function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n;
  let u = -1;
  do {
    v /= 1024;
    u += 1;
  } while (v >= 1024 && u < units.length - 1);
  return `${v.toFixed(1)} ${units[u]}`;
}

// 秒 → 人类可读（秒/分钟/小时）
function fmtSec(secs: number): string {
  if (secs < 60) return `${Math.ceil(secs)}s`;
  if (secs < 3600) return `${Math.ceil(secs / 60)}m`;
  return `${(secs / 3600).toFixed(1)}h`;
}

// 连接模式徽标文案与样式（direct→绿 / relay→橙 / punching→蓝）
const CONN_META: Record<string, { label: string; cls: string }> = {
  direct: { label: "P2P直连", cls: "badge-direct" },
  relay: { label: "中继relay", cls: "badge-relay" },
  punching: { label: "打洞中", cls: "badge-punching" },
};

export default function TransferPanel({ name, events, log, onCancel }: Props) {
  // 取各类事件的"最新一条"驱动面板（conn/progress/resume 各保留最新；done/error 即终态）
  const { conn, progress, resume, terminal } = useMemo(() => {
    let conn: TransferEvent | null = null;
    let progress: TransferEvent | null = null;
    let resume: TransferEvent | null = null;
    let terminal: TransferEvent | null = null;
    for (const ev of events) {
      if (ev.type === "conn") conn = ev;
      else if (ev.type === "progress") progress = ev;
      else if (ev.type === "resume") resume = ev;
      else terminal = ev; // done / error（终态，后者覆盖前者）
    }
    return { conn, progress, resume, terminal };
  }, [events]);

  // 速度估算：记录最近几次 progress 事件的 sent + 到达时刻，用于剩余时间。
  // GUI-T3 修复：新传输（事件流从空开始，DatasetsPage 在 startShare/startReceive
  // 时清空 events）重置样本，避免旧传输的慢速/快速样本让新传输 ETA 短期失真。
  const speedRef = useRef<{ sent: number; t: number }[]>([]);
  useEffect(() => {
    const ev = events[events.length - 1];
    if (events.length === 0) {
      speedRef.current = [];
    } else if (ev?.type === "progress") {
      speedRef.current = [
        ...speedRef.current.slice(-8),
        { sent: ev.sent, t: performance.now() },
      ];
    }
  }, [events]);

  // 剩余时间估计（无足够样本 / 未开始时不显示）
  let etaSec: number | null = null;
  if (progress && !terminal && progress.total > 0) {
    const pts = speedRef.current;
    if (pts.length >= 2) {
      const first = pts[0];
      const lastP = pts[pts.length - 1];
      const dt = (lastP.t - first.t) / 1000;
      const ds = lastP.sent - first.sent;
      if (dt >= 0.5 && ds > 0) {
        etaSec = (progress.total - progress.sent) / (ds / dt);
      }
    }
  }

  // 派生速度（B/s），与 ETA 共享同一份样本
  let speedBps: number | null = null;
  if (progress && !terminal) {
    const pts = speedRef.current;
    if (pts.length >= 2) {
      const first = pts[0];
      const lastP = pts[pts.length - 1];
      const dt = (lastP.t - first.t) / 1000;
      const ds = lastP.sent - first.sent;
      if (dt >= 0.5 && ds > 0) {
        speedBps = ds / dt;
      }
    }
  }

  // 状态文案（终态优先，其次连接模式，最后默认"正在连接"）
  let statusText: string;
  if (terminal) {
    if (terminal.type === "done") {
      statusText = terminal.ok
        ? `✅ 传输完成（共 ${fmtBytes(terminal.bytes)}，用时 ${Math.round(terminal.elapsed_secs)} 秒）`
        : "❌ 传输失败或已取消";
    } else {
      statusText = `⚠️ ${terminal.msg}`;
    }
  } else if (conn) {
    statusText =
      conn.mode === "punching"
        ? "🔗 打洞中，正在建立 P2P 直连…"
        : conn.mode === "relay"
          ? "🔄 已使用中继 relay 传输"
          : "⚡ P2P 直连传输中";
  } else {
    statusText = "⏳ 正在连接…";
  }

  // 进度条：最新 progress 的 pct（无事件时为 0）
  const pct = progress ? Math.max(0, Math.min(100, progress.pct)) : 0;
  const done = progress?.sent ?? 0;
  const total = progress?.total ?? 0;

  // 模式徽标（direct/relay/punching → 不同色调 token）
  let modeBadge: React.ReactNode = null;
  if (conn) {
    const meta = CONN_META[conn.mode];
    const label = meta?.label ?? conn.mode;
    const cls = meta?.cls ?? "badge-other";
    let bg = "hsl(var(--primary))";
    let fg = "hsl(var(--primary-fg, var(--bg)))";
    if (conn.mode === "relay") {
      bg = "hsl(var(--warning, 35 90% 55%))";
      fg = "hsl(var(--warning-fg, 35 90% 15%))";
    } else if (conn.mode === "punching") {
      bg = "hsl(var(--accent, 210 90% 55%))";
      fg = "hsl(var(--accent-fg, 210 90% 15%))";
    }
    modeBadge = (
      <span
        className={`badge ${cls}`}
        style={{
          background: bg,
          color: fg,
          fontSize: 11,
          fontWeight: 600,
          padding: "2px 8px",
          borderRadius: 999,
        }}
      >
        {label}
      </span>
    );
  }

  // 续传徽标
  let resumeBadge: React.ReactNode = null;
  if (resume) {
    const pct = Math.round((resume.done / resume.total) * 100);
    resumeBadge = (
      <span
        className="badge badge-resume"
        style={{
          background: "hsl(var(--muted, var(--border)))",
          color: "hsl(var(--fg-muted))",
          fontSize: 11,
          fontWeight: 600,
          padding: "2px 8px",
          borderRadius: 999,
        }}
      >
        已恢复 {pct}%
      </span>
    );
  }

  return (
    <div className="transfer-panel">
      <div className="card" style={{ marginTop: 10 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginBottom: 8,
          }}
        >
          <span style={{ fontSize: 14, fontWeight: 600, color: "hsl(var(--fg))" }}>
            {name}
          </span>
          {modeBadge}
          {resumeBadge}
        </div>

        {progress != null && (
          <>
            <div
              role="progressbar"
              aria-valuenow={Math.round(pct)}
              aria-valuemin={0}
              aria-valuemax={100}
              style={{
                position: "relative",
                height: 8,
                background: "hsl(var(--border))",
                borderRadius: 999,
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  width: `${pct}%`,
                  height: "100%",
                  background: "hsl(var(--primary))",
                  transition: "width 200ms ease",
                }}
              />
            </div>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                marginTop: 4,
                fontSize: 12,
                color: "hsl(var(--fg-muted))",
                fontVariantNumeric: "tabular-nums",
              }}
            >
              <span>
                {fmtBytes(done)} / {fmtBytes(total)} · {pct.toFixed(1)}%
              </span>
              {speedBps != null && (
                <span>
                  {fmtBytes(speedBps)}/s
                  {etaSec != null && ` · ETA ${fmtSec(etaSec)}`}
                </span>
              )}
            </div>
          </>
        )}
      </div>
      <div className="transfer-meta">
        {progress ? `${fmtBytes(progress.sent)} / ${fmtBytes(progress.total)}（${pct}%） · ` : ""}
        {statusText}
      </div>
      <div className="transfer-actions">
        <button className="secondary" disabled={!!terminal} onClick={onCancel}>
          ✖ 取消传输
        </button>
      </div>
      {log.length > 0 && (
        <details className="transfer-log">
          <summary>原始日志（{log.length} 行）</summary>
          {log.map((l, i) => (
            <div key={i}>{l}</div>
          ))}
        </details>
      )}
    </div>
  );
}
