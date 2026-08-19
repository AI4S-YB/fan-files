import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { TransferEvent } from "../components/TransferPanel";

// 共享状态：idle=未共享，code=已生成码等待接收，running=传输中，
// done=完成/失败，cancelled=用户取消（后端 done(-1) 已由取消流程消费，忽略）
export type ShareState =
  | { status: "idle" }
  | { status: "running" }
  | { status: "code"; code: string }
  | { status: "done"; ok: boolean }
  | { status: "cancelled" };

// 续传确认弹窗内容（share://progress 的 resume 事件触发）
export interface ShareResumeAsk {
  done: number;
  total: number;
}

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

// GUI-T5 修复：共享状态与 share:// 监听从 DatasetDetailModal 提升到页面级——
// 监听随页面存活，关闭弹层不再丢跟踪（子进程传输期间页面保留进度/取消入口）。
// 数据集页/搜索页各持一份实例；引擎同一时刻只跑一个传输，跨弹层打开时
// 共享面板仍指向发起共享的数据集（shareName 在 startShare 时固化）。
// onDone：传输结束（done/error，非用户取消）时回调，DatasetsPage 用于刷新传输历史。
export function useShareTransfer(options?: { onDone?: (ok: boolean) => void }) {
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
  // 发起共享的数据集名（TransferPanel 标题；弹层关闭后仍指向原数据集）
  const [shareName, setShareName] = useState("");
  // 续传确认弹窗（resume 事件触发）
  const [shareResume, setShareResume] = useState<ShareResumeAsk | null>(null);
  // 续传弹窗超时（规格 §九：用户不响应默认继续——引擎已自动续传，不阻塞传输）
  const RESUME_AUTO_CLOSE_MS = 60_000;
  const resumeTimerRef = useRef<number | null>(null);
  // onDone 存 ref：监听器只在挂载时订阅一次，回调每次渲染取最新（避免旧闭包）
  const onDoneRef = useRef(options?.onDone);
  onDoneRef.current = options?.onDone;

  // 弹窗显示的同时挂 60s 自动关闭定时器（重复触发时重置计时）
  function showResumeAsk(ask: ShareResumeAsk) {
    if (resumeTimerRef.current) window.clearTimeout(resumeTimerRef.current);
    setShareResume(ask);
    resumeTimerRef.current = window.setTimeout(
      () => setShareResume(null),
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

  // 监听共享事件流（share://code / progress / done / error）。页面挂载即监听、
  // 卸载即清理（不随弹层开关）——progress 为 JSONL 行：JSON.parse 成功 → 分发到面板；
  // 失败（人类输出等）→ 仅进原始日志。
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
      const ok = e.payload === 0;
      setShareState({ status: "done", ok });
      setShareRaw((l) => [
        ...l.slice(-200),
        ok ? "共享完成" : `共享失败（退出码 ${e.payload}）`,
      ]);
      onDoneRef.current?.(ok);
    });
    const unError = listen<string>("share://error", (e) => {
      setShareState({ status: "done", ok: false });
      setShareRaw((l) => [...l.slice(-200), `共享错误: ${e.payload}`]);
      onDoneRef.current?.(false);
    });
    return () => {
      unCode.then((u) => u());
      unProgress.then((u) => u());
      unDone.then((u) => u());
      unError.then((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function startShare(path: string, name: string) {
    setShareName(name);
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
    setShareResume(null);
  }

  function rejectResume() {
    continueResume();
    void cancelShare();
  }

  return {
    share,
    shareEvents,
    shareRaw,
    shareName,
    shareResume,
    startShare,
    cancelShare,
    continueResume,
    rejectResume,
  };
}
