import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  fetchDatasets,
  fetchDatasetDetail,
  fetchFiles,
  type DatasetSummary,
  type DatasetDetail,
  type FileSummary,
  type Facet,
} from "../api";
import DataTable from "../components/DataTable";
import DatasetDetailModal from "../components/DatasetDetailModal";
import TransferPanel, { type TransferEvent } from "../components/TransferPanel";

// meta.type_counts 缺失时（老后端/空库）回退到固定类型集
const FALLBACK_TYPES = ["genome", "transcriptome", "variant", "other"];

// 解析引擎 JSONL 事件行（接收侧）；非 JSON 行（人类输出）返回 null（进原始日志）
function parseTransferLine(line: string): TransferEvent | null {
  try {
    const obj = JSON.parse(line) as TransferEvent;
    if (obj && typeof obj === "object" && typeof obj.type === "string") return obj;
  } catch {
    /* 非 JSON 行忽略 */
  }
  return null;
}

// 数据集页排序下拉选项（sort 参数值；order 恒为 asc —— 服务端仅支持 asc）
type SortValue = "id" | "name" | "file_count";

type ReceiveStatus = "idle" | "running" | "done-ok" | "done-err" | "cancelled";

// 续传确认弹窗内容（receive://progress 的 resume 事件触发；共享侧的在 DatasetDetailModal）
interface ResumeAsk {
  done: number;
  total: number;
}

export default function DatasetsPage() {
  const [rows, setRows] = useState<DatasetSummary[]>([]);
  const [nextCursor, setNextCursor] = useState<number | null>(null);
  const [type, setType] = useState<string | undefined>(undefined);
  const [typeCounts, setTypeCounts] = useState<Facet[]>([]);
  // GUI-T4: 搜索（提交后生效的 q，与类型筛选共存）+ 排序下拉
  const [q, setQ] = useState("");
  const [submittedQ, setSubmittedQ] = useState<string | undefined>(undefined);
  const [sort, setSort] = useState<SortValue>("id");
  const [detail, setDetail] = useState<DatasetDetail | null>(null);
  const [files, setFiles] = useState<FileSummary[]>([]);
  // 已访问页使用的 cursor 栈：next 时 push 当前 nextCursor，prev 时 pop 并 load 新栈顶（栈空即回第一页）
  const [history, setHistory] = useState<number[]>([]);
  // 翻页 loading guard：请求在途时禁用上一页/下一页，防连点双请求
  const [loading, setLoading] = useState(false);
  // 接收（P2P）状态
  const [receiveCode, setReceiveCode] = useState("");
  const [receiveStatus, setReceiveStatus] = useState<ReceiveStatus>("idle");
  const receiveStatusRef = useRef<ReceiveStatus>("idle");
  const setReceiveState = (s: ReceiveStatus) => {
    receiveStatusRef.current = s;
    setReceiveStatus(s);
  };
  const [receiveEvents, setReceiveEvents] = useState<TransferEvent[]>([]);
  const [receiveRaw, setReceiveRaw] = useState<string[]>([]);
  // 最近一次接收的目标目录（"打开接收目录"用；来自 receive_dataset 返回的实际路径）
  const [receiveDir, setReceiveDir] = useState<string | null>(null);
  // 续传确认弹窗（接收侧 resume 事件触发）
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
  // 传输历史
  const [transferHistory, setTransferHistory] = useState<HistoryEntry[]>([]);

  interface HistoryEntry {
    direction: string;
    dataset: string;
    code: string;
    status: string;
    bytes_sent: number;
    bytes_received: number;
    time: number;
  }

  async function loadHistory() {
    try {
      const h = await invoke<HistoryEntry[]>("transfer_history");
      setTransferHistory(h ?? []);
    } catch {
      setTransferHistory([]);
    }
  }

  // 挂载时加载历史；接收完成后刷新（共享完成的刷新在弹层内）
  useEffect(() => {
    void loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [receiveStatus === "done-ok"]);

  // 监听接收事件流（receive://progress / done / error）。progress 为 JSONL 行：
  // JSON.parse 成功 → 分发到面板；失败（人类输出等）→ 仅进原始日志。
  useEffect(() => {
    const unRProgress = listen<string>("receive://progress", (e) => {
      setReceiveRaw((l) => [...l.slice(-200), e.payload]);
      const ev = parseTransferLine(e.payload);
      if (!ev) return;
      setReceiveEvents((es) => [...es.slice(-200), ev]);
      if (ev.type === "resume") {
        showResumeAsk({ done: ev.done, total: ev.total });
      }
    });
    const unRDone = listen<number>("receive://done", (e) => {
      const s = receiveStatusRef.current;
      if (s === "cancelled" || s === "idle") return;
      setReceiveState(e.payload === 0 ? "done-ok" : "done-err");
      setReceiveRaw((l) => [
        ...l.slice(-200),
        e.payload === 0 ? "接收完成" : `接收失败（退出码 ${e.payload}）`,
      ]);
    });
    const unRError = listen<string>("receive://error", (e) => {
      setReceiveState("done-err");
      setReceiveRaw((l) => [...l.slice(-200), `接收错误: ${e.payload}`]);
    });
    return () => {
      unRProgress.then((u) => u());
      unRDone.then((u) => u());
      unRError.then((u) => u());
    };
  }, []);

  async function startReceive() {
    const code = receiveCode.trim();
    if (!code || receiveStatus === "running") return;
    setReceiveState("running");
    setReceiveEvents([]);
    setReceiveRaw([]);
    try {
      // GUI-T3 修复：不传 output，接收目录由后端 config [transfer].receive_dir 决定
      // （未配置时后端回退 ~/Downloads/fan-received）；命令返回实际目录供"打开"用
      const dir = await invoke<string>("receive_dataset", { code });
      setReceiveDir(dir);
    } catch (e) {
      setReceiveState("done-err");
      setReceiveRaw((l) => [...l, `接收启动失败: ${String(e)}`]);
    }
  }

  async function cancelReceive() {
    const s = receiveStatusRef.current;
    if (s === "idle" || s === "cancelled") return;
    setReceiveState("cancelled");
    setReceiveEvents((es) => [
      ...es.slice(-200),
      { type: "done", ok: false, bytes: 0, elapsed_secs: 0 },
    ]);
    setReceiveRaw((l) => [...l.slice(-200), "已取消"]);
    try {
      await invoke("cancel_transfer");
    } catch {
      /* 取消失败不影响面板终态 */
    }
  }

  // 续传确认：继续 → 仅关闭弹窗（引擎已自动续传缺失块）；放弃 → 取消接收
  function continueResume() {
    if (resumeTimerRef.current) window.clearTimeout(resumeTimerRef.current);
    resumeTimerRef.current = null;
    setResumeAsk(null);
  }

  function rejectResume() {
    continueResume();
    void cancelReceive();
  }

  function openReceiveDir() {
    if (receiveDir) {
      invoke("open_path", { path: receiveDir }).catch(console.error);
    }
  }

  // 游标分页：next_cursor 非空则"下一页"可用（cursor 即上一页最后一条的 id）。
  // 错误在内部消化（不向外 reject），失败时清空行/游标/历史栈。
  // GUI-T4: 带上搜索词与排序键（sort=id 时省略 sort/order，服务端默认 id 升序）。
  async function load(
    cursor?: number,
    selectedType?: string,
    query?: string,
    sortValue: SortValue = "id"
  ) {
    setLoading(true);
    try {
      const page = await fetchDatasets({
        cursor,
        limit: 50,
        type: selectedType,
        q: query,
        sort: sortValue === "id" ? undefined : sortValue,
        order: sortValue === "id" ? undefined : "asc",
      });
      setRows(page.data);
      setNextCursor(page.meta.next_cursor);
      if (page.meta.type_counts) setTypeCounts(page.meta.type_counts);
    } catch {
      setRows([]);
      setNextCursor(null);
      setHistory([]);
    } finally {
      setLoading(false);
    }
  }

  // 搜索词/排序/类型筛选任一变化：清历史栈回第一页重新加载（防竞态：cursor 回开头）
  useEffect(() => {
    setHistory([]);
    void load(undefined, type, submittedQ, sort);
  }, [type, submittedQ, sort]);

  // 提交搜索：空白词视作"无过滤"（q 省略）；与已提交词相同则跳过重复请求
  function submitSearch() {
    const next = q.trim() || undefined;
    if (next !== submittedQ) setSubmittedQ(next);
  }

  function goNext() {
    if (loading || !nextCursor) return;
    setHistory((h) => [...h, nextCursor]);
    void load(nextCursor, type, submittedQ, sort);
  }

  function goPrev() {
    if (loading || history.length === 0) return;
    // pop 栈顶（当前页的 cursor），load 新栈顶即上一页的 cursor；栈空为 undefined 回第一页
    const prevTop = history.length > 1 ? history[history.length - 2] : undefined;
    setHistory((h) => h.slice(0, -1));
    void load(prevTop, type, submittedQ, sort);
  }

  async function openDetail(r: DatasetSummary) {
    setFiles([]);
    let d: DatasetDetail;
    try {
      d = await fetchDatasetDetail(r.id);
    } catch {
      return; // 详情加载失败静默返回（T15 全局错误横幅接管）
    }
    setDetail(d);
    fetchFiles(r.id)
      .then((page) => setFiles(page.data))
      .catch(() => setFiles([])); // 文件列表失败静默为空
  }

  // 优先用后端聚合的 type_counts 的键（Facet.value），缺失时回退固定集
  const chips = typeCounts.length > 0 ? typeCounts.map((f) => f.value) : FALLBACK_TYPES;
  const countOf = (t: string) => typeCounts.find((f) => f.value === t)?.count;

  return (
    <div className="page">
      <h2>数据集</h2>
      {/* P2P 接收入口：输入对方发来的配对码接收数据 */}
      <div className="receive-bar">
        <input
          className="receive-input"
          placeholder="📥 输入配对码接收数据（如 8-purple-hammer）"
          value={receiveCode}
          onChange={(e) => setReceiveCode(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && startReceive()}
        />
        <button
          className="primary"
          disabled={receiveStatus === "running" || !receiveCode.trim()}
          onClick={startReceive}
        >
          {receiveStatus === "running" ? "接收中…" : "接收"}
        </button>
        {receiveStatus === "done-ok" && (
          <span className="feedback-ok">✅ 已接收</span>
        )}
        {receiveStatus === "done-ok" && (
          <button className="secondary" onClick={() => openReceiveDir()}>
            📂 打开接收目录
          </button>
        )}
        {receiveStatus === "done-err" && (
          <span className="feedback-err">❌ 接收失败</span>
        )}
        {receiveStatus === "cancelled" && (
          <span className="feedback-err">已取消</span>
        )}
        {/* 接收传输面板（进度/徽标/续传/取消 + 折叠原始日志） */}
        {receiveStatus !== "idle" && (
          <div className="receive-panel-wrap">
            <TransferPanel
              name={receiveCode.trim() || "接收"}
              events={receiveEvents}
              log={receiveRaw}
              onCancel={() => void cancelReceive()}
            />
          </div>
        )}
      </div>
      {/* P2P 传输历史 */}
      {transferHistory.length > 0 && (
        <details className="history-panel">
          <summary>🕘 传输历史（{transferHistory.length}）</summary>
          <table className="history-table">
            <thead>
              <tr>
                <th>时间</th>
                <th>方向</th>
                <th>数据集/码</th>
                <th>状态</th>
                <th>字节</th>
              </tr>
            </thead>
            <tbody>
              {transferHistory.map((h, i) => (
                <tr key={i}>
                  <td className="mono">{new Date(h.time * 1000).toLocaleString()}</td>
                  <td>{h.direction === "send" ? "📤 发送" : "📥 接收"}</td>
                  <td className="mono">{h.dataset}</td>
                  <td>
                    <span className={h.status === "ok" ? "badge badge-other" : "feedback-err"}>
                      {h.status === "ok" ? "✓ 成功" : "✗ 失败"}
                    </span>
                  </td>
                  <td className="mono">{(h.bytes_sent + h.bytes_received).toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
      {/* GUI-T4: 搜索框（q 参数，名称/关键词过滤）+ 排序下拉（name/file_count） */}
      <div className="dataset-toolbar">
        <div className="dataset-search">
          <input
            className="search-box"
            placeholder="搜索名称/关键词…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submitSearch()}
            aria-label="搜索数据集"
          />
          <button className="primary" disabled={loading} onClick={submitSearch}>
            搜索
          </button>
        </div>
        <select
          className="sort-select"
          aria-label="排序方式"
          value={sort}
          disabled={loading}
          onChange={(e) => setSort(e.target.value as SortValue)}
        >
          <option value="id">默认排序</option>
          <option value="name">按名称</option>
          <option value="file_count">按文件数</option>
        </select>
      </div>
      <div className="filters">
        {chips.map((t) => (
          <button
            key={t}
            className={type === t ? "chip active" : "chip"}
            // 翻页请求在途时禁用筛选，防止"切筛选 → 旧响应后到覆盖 UI"的竞态
            disabled={loading}
            onClick={() => setType(type === t ? undefined : t)}
          >
            {countOf(t) != null ? `${t} (${countOf(t)})` : t}
          </button>
        ))}
      </div>
      <DataTable rows={rows} onSelect={openDetail} />
      <div className="pager">
        <button disabled={loading || history.length === 0} onClick={goPrev}>
          上一页
        </button>
        <button disabled={loading || !nextCursor} onClick={goNext}>
          下一页
        </button>
      </div>
      {/* GUI-T4: 详情弹层提取为共享组件（详情+资产+文件+共享按钮，share:// 逻辑自持） */}
      {detail && (
        <DatasetDetailModal detail={detail} files={files} onClose={() => setDetail(null)} />
      )}
      {/* 续传确认弹窗（接收侧 resume 事件触发；继续=关弹窗，引擎已自动续传） */}
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
  );
}
