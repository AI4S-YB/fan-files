import { useState } from "react";
import TransferPanel, { type TransferEvent } from "./TransferPanel";

// NR-T5: 有效期提示文案用实际选择值（不再硬编码 7 天）。
// 整天数（24 的倍数）→ "N 天内有效"（168 → "7 天内有效"），其余 → "N 小时内有效"。
export function ttlTip(hours: number): string {
  if (hours > 0 && hours % 24 === 0) return `${hours / 24} 天内有效`;
  return `${hours} 小时内有效`;
}

interface Props {
  name: string; // 数据集名（传输面板标题）
  code?: string; // 配对码（share.status === "code" 时展示）
  events: TransferEvent[]; // 已解析事件流（页面级 useShareTransfer 分发后传入）
  log: string[]; // 原始行（含非 JSON 的人类输出，失败原因可见）
  onCancel: () => void; // 取消当前共享
  ttlHours?: number; // 配对码有效期（小时）；缺省 168 = 引擎默认 7 天
}

// GUI-T5: 从 DatasetDetailModal 提取的共享面板（配对码 + 传输面板），
// 弹层内与页面级共用——页面级实例保证弹层关闭后传输仍可跟踪/取消。
export default function SharePanel({ name, code, events, log, onCancel, ttlHours: _ttlHours = 168 }: Props) {
  // 配对码复制反馈
  const [copied, setCopied] = useState(false);

  return (
    <div className="share-panel">
      {code && (
        <div
          className="anim-fade-up"
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: 12,
            padding: 20,
            background: "hsl(var(--primary) / .06)",
            border: "1px solid hsl(var(--primary) / .2)",
            borderRadius: "var(--radius-lg)",
          }}
        >
          <div
            style={{
              fontSize: 11,
              color: "hsl(var(--fg-muted))",
              textTransform: "uppercase",
              letterSpacing: "0.1em",
              fontWeight: 600,
            }}
          >
            配对码
          </div>
          <div
            className="anim-pulse-ring"
            style={{
              fontSize: 28,
              fontFamily: "var(--font-mono)",
              fontWeight: 700,
              color: "hsl(var(--primary))",
              letterSpacing: "0.05em",
              padding: "8px 16px",
              borderRadius: "var(--radius)",
              background: "hsl(var(--bg))",
              border: "1px solid hsl(var(--primary) / .3)",
            }}
          >
            {code}
          </div>
          <button
            type="button"
            className="btn btn-secondary transition-base"
            onClick={() => {
              navigator.clipboard.writeText(code);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
            aria-label="复制配对码"
          >
            {copied ? "✓ 已复制" : "📋 复制"}
          </button>
        </div>
      )}
      {/* 共享传输面板（进度/徽标/续传/取消 + 折叠原始日志） */}
      <TransferPanel name={name} events={events} log={log} onCancel={onCancel} />
    </div>
  );
}
