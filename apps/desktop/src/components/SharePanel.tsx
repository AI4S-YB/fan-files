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
export default function SharePanel({ name, code, events, log, onCancel, ttlHours = 168 }: Props) {
  // 配对码复制反馈
  const [copied, setCopied] = useState(false);

  // 复制配对码到剪贴板（navigator.clipboard；非安全上下文等失败静默）
  async function copyCode() {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      /* 剪贴板不可用时静默 */
    }
  }

  return (
    <div className="share-panel">
      {code && (
        <div className="share-code">
          <div className="share-code-label">把下面的配对码发给对方，对方执行：</div>
          <div className="share-code-row">
            <code className="share-code-value">{code}</code>
            <button className="secondary copy-btn" onClick={copyCode}>
              {copied ? "已复制 ✓" : "📋 复制"}
            </button>
          </div>
          <div className="share-code-cmd">
            fan-files transfer get {code}
          </div>
          {/* NR-T4/T5 文案同步：配对码有效期提示用实际选择值（引擎默认 7 天） */}
          <div className="share-code-tip">⏳ 配对码 {ttlTip(ttlHours)}</div>
        </div>
      )}
      {/* 共享传输面板（进度/徽标/续传/取消 + 折叠原始日志） */}
      <TransferPanel name={name} events={events} log={log} onCancel={onCancel} />
    </div>
  );
}
