import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DatasetDetail, FileSummary } from "../api";
import TransferPanel, { type TransferEvent } from "./TransferPanel";

// 解析引擎 JSONL 事件行；非 JSON 行（人类输出/配对码）返回 null（进原始日志）
function parseTransferLine(line: string): TransferEvent | null {
  try {
    const obj = JSON.parse(line) as TransferEvent;
    if (obj && typeof obj === "object" && typeof obj.type === "string") return obj;
  } catch {
    /* 非 JSON 行忽略 */
  }
  return null;
}

// 共享状态：idle=未共享，code=已生成码等待接收，running=传输中，
// done=完成/失败，cancelled=用户取消（后端 done(-1) 已由取消流程消费，忽略）
type ShareState =
  | { status: "idle" }
  | { status: "running" }
  | { status: "code"; code: string }
  | { status: "done"; ok: boolean }
  | { status: "cancelled" };

// 续传确认弹窗内容（share://progress 的 resume 事件触发；接收侧的在 DatasetsPage）
interface ResumeAsk {
  done: number;
  total: number;
}

// GUI-T4: 从 DatasetsPage 弹层提取为共享组件（数据集详情 + 资产列表 + 文件列表 +
// 共享按钮）。共享状态与 share:// 事件监听随弹层自持（挂载即监听、卸载即清理），
// 两个页面（数据集/搜索）都以 detail+files+onClose 三 props 复用，无需复制共享逻辑。
export default function DatasetDetailModal({
  detail,
  files,
  onClose,
}: {
  detail: DatasetDetail;
  files: FileSummary[];
  onClose: () => void;
}) {
  // 数据集共享（P2P）状态
  const [share, setShare] = useState<ShareState>({ status: "idle" });
  // ref 镜像：事件监听闭包需读"当前"状态（取消后忽略后续 done(-1)，避免覆盖 cancelled 态）
  const shareStatusRef = useRef<ShareState>({ status: "idle" });
  const setShareState = (s: ShareState) => {
    shareStatusRef.current = s;
    setShare(s);
  };
  // 共享面板：解析后的事件（驱动面板）+ 原始行（折叠日志）
  const [shareEvents, setShareEvents] = useState<TransferEvent[]>([]);
  const [shareRaw, setShareRaw] = useState<string[]>([]);
  // 配对码复制反馈
  const [copied, setCopied] = useState(false);
  // 续传确认弹窗（resume 事件触发）
  const [resumeAsk, setResumeAsk] = useState<ResumeAsk | null>(null);
  // 续传弹窗超时（规格 §九：用户不响应默认继续——引擎已自动续传，不阻塞传输）
  const RESUME_AUTO_CLOSE_MS = 60_000;
  const resumeTimerRef = useRef<number | null>(null);
  // 弹窗显示的同时挂 60s 自动关闭定时器（重复触发时重置计时）
  function showResumeAsk(ask: ResumeAsk) {
    if (resumeTimerRef.current) window.clearTimeout(resumeTimerRef.current);
    setResumeAsk(ask);
    resumeTimerRef.current = window.setTimeout(
      () => setResumeAsk(null),
      RESUME_AUTO_CLOSE_MS
    );
  }
  // 卸载时清理弹窗定时器
  useEffect(
    () => () => {
      if (resumeTimerRef.current) window.clearTimeout(resumeTimerRef.current);
    },
    []
  );

  // 监听共享事件流（share://code / progress / done / error）。弹层挂载即监听、
  // 卸载即清理；progress 为 JSONL 行：JSON.parse 成功 → 分发到面板；失败（人类输出等）
  // → 仅进原始日志。
  useEffect(() => {
    const unCode = listen<string>("share://code", (e) => {
      setShareState({ status: "code", code: e.payload });
      setShareRaw((l) => [...l.slice(-200), `配对码: ${e.payload}`]);
    });
    const unProgress = listen<string>("share://progress", (e) => {
      setShareRaw((l) => [...l.slice(-200), e.payload]);
      const ev = parseTransferLine(e.payload);
      if (!ev) return;
      setShareEvents((es) => [...es.slice(-200), ev]);
      if (ev.type === "resume") {
        showResumeAsk({ done: ev.done, total: ev.total });
      }
    });
    const unDone = listen<number>("share://done", (e) => {
      const s = shareStatusRef.current;
      if (s.status === "cancelled" || s.status === "idle") return;
      setShareState({ status: "done", ok: e.payload === 0 });
      setShareRaw((l) => [
        ...l.slice(-200),
        e.payload === 0 ? "共享完成" : `共享失败（退出码 ${e.payload}）`,
      ]);
    });
    const unError = listen<string>("share://error", (e) => {
      setShareState({ status: "done", ok: false });
      setShareRaw((l) => [...l.slice(-200), `共享错误: ${e.payload}`]);
    });
    return () => {
      unCode.then((u) => u());
      unProgress.then((u) => u());
      unDone.then((u) => u());
      unError.then((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function startShare(path: string) {
    setShareState({ status: "running" });
    setShareEvents([]);
    setShareRaw([]);
    try {
      await invoke("share_dataset", { path });
    } catch (e) {
      setShareState({ status: "done", ok: false });
      setShareRaw((l) => [...l, `共享启动失败: ${String(e)}`]);
    }
  }

  // 取消共享：面板推进到终态（合成 done 事件 → 取消按钮禁用），后端杀子进程；
  // 后端随后发的 done(-1) 由监听闭包按 cancelled 态忽略
  async function cancelShare() {
    const s = shareStatusRef.current;
    if (s.status === "idle" || s.status === "cancelled") return;
    setShareState({ status: "cancelled" });
    setShareEvents((es) => [
      ...es.slice(-200),
      { type: "done", ok: false, bytes: 0, elapsed_secs: 0 },
    ]);
    setShareRaw((l) => [...l.slice(-200), "已取消"]);
    try {
      await invoke("cancel_transfer");
    } catch {
      /* 取消失败不影响面板终态 */
    }
  }

  // 续传确认：继续 → 仅关闭弹窗（引擎已自动续传缺失块）；放弃 → 取消共享
  function continueResume() {
    if (resumeTimerRef.current) window.clearTimeout(resumeTimerRef.current);
    resumeTimerRef.current = null;
    setResumeAsk(null);
  }

  function rejectResume() {
    continueResume();
    void cancelShare();
  }

  // 复制配对码到剪贴板（navigator.clipboard；非安全上下文等失败静默）
  async function copyCode() {
    if (share.status !== "code") return;
    try {
      await navigator.clipboard.writeText(share.code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      /* 剪贴板不可用时静默 */
    }
  }

  return (
    <div className="modal" onClick={onClose}>
      <div className="modal-body" onClick={(e) => e.stopPropagation()}>
        <h3>{detail.name}</h3>
        <p>
          物种: {detail.species ?? "—"} · 路径: {detail.path ?? "—"}
        </p>
        <h4>资产</h4>
        <ul>
          {detail.assets.map((a) => (
            <li key={a.id}>
              {a.name ?? "—"}（{a.type ?? "—"}）· {a.file_count} 文件
            </li>
          ))}
        </ul>
        <h4>文件</h4>
        <ul className="file-list">
          {files.slice(0, 20).map((f) => (
            <li key={f.id}>{f.path ?? f.name}</li>
          ))}
        </ul>
        <div className="modal-actions">
          <button
            disabled={!detail.path || share.status === "running"}
            title={detail.path ? "生成配对码，对方凭码接收" : "无本地路径"}
            onClick={() => detail.path && startShare(detail.path)}
          >
            📤 共享
          </button>
          {/* T13: 系统文件管理器打开数据集目录；无本地路径时保持禁用 */}
          <button
            disabled={!detail.path}
            title={detail.path ? undefined : "无本地路径"}
            onClick={() =>
              detail.path &&
              invoke("open_path", { path: detail.path }).catch(console.error)
            }
          >
            📂 打开目录
          </button>
        </div>
        {share.status !== "idle" && (
          <div className="share-panel">
            {share.status === "code" && (
              <div className="share-code">
                <div className="share-code-label">把下面的配对码发给对方，对方执行：</div>
                <div className="share-code-row">
                  <code className="share-code-value">{share.code}</code>
                  <button className="secondary copy-btn" onClick={copyCode}>
                    {copied ? "已复制 ✓" : "📋 复制"}
                  </button>
                </div>
                <div className="share-code-cmd">
                  fan-files transfer get {share.code}
                </div>
                {/* Minor-4 已知偏差（不改）："24 小时内有效"为硬编码，
                    与引擎配对码默认有效期 24h 一致（transfer.rs CODE_TTL）；
                    引擎若改默认需同步此处文案 */}
                <div className="share-code-tip">⏳ 配对码 24 小时内有效</div>
              </div>
            )}
            {/* 共享传输面板（进度/徽标/续传/取消 + 折叠原始日志） */}
            <TransferPanel
              name={detail.name}
              events={shareEvents}
              log={shareRaw}
              onCancel={() => void cancelShare()}
            />
          </div>
        )}
        {/* 续传确认弹窗（share://progress resume 事件触发；继续=关弹窗，引擎已自动续传） */}
        {resumeAsk && (
          <div className="modal" onClick={continueResume}>
            <div className="modal-body" onClick={(e) => e.stopPropagation()}>
              <h3>续传确认</h3>
              <p>
                发现未完成传输，已收 {resumeAsk.done}/{resumeAsk.total}（
                {Math.round((resumeAsk.done / resumeAsk.total) * 100)}%），是否续传？
              </p>
              <div className="modal-actions">
                <button className="primary" onClick={continueResume}>
                  继续续传
                </button>
                <button className="secondary" onClick={rejectResume}>
                  放弃并取消
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
