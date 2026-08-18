import { useEffect, useState } from "react";
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

// meta.type_counts 缺失时（老后端/空库）回退到固定类型集
const FALLBACK_TYPES = ["genome", "transcriptome", "variant", "other"];

// 共享状态：null=未共享，code=已生成码等待接收，running=传输中，done=完成
type ShareState =
  | { status: "idle" }
  | { status: "running" }
  | { status: "code"; code: string }
  | { status: "done"; ok: boolean };

export default function DatasetsPage() {
  const [rows, setRows] = useState<DatasetSummary[]>([]);
  const [nextCursor, setNextCursor] = useState<number | null>(null);
  const [type, setType] = useState<string | undefined>(undefined);
  const [typeCounts, setTypeCounts] = useState<Facet[]>([]);
  const [detail, setDetail] = useState<DatasetDetail | null>(null);
  const [files, setFiles] = useState<FileSummary[]>([]);
  // 已访问页使用的 cursor 栈：next 时 push 当前 nextCursor，prev 时 pop 并 load 新栈顶（栈空即回第一页）
  const [history, setHistory] = useState<number[]>([]);
  // 翻页 loading guard：请求在途时禁用上一页/下一页，防连点双请求
  const [loading, setLoading] = useState(false);
  // 数据集共享（P2P）状态 + 进度日志
  const [share, setShare] = useState<ShareState>({ status: "idle" });
  const [shareLog, setShareLog] = useState<string[]>([]);
  // 接收（P2P）状态
  const [receiveCode, setReceiveCode] = useState("");
  const [receiveStatus, setReceiveStatus] = useState<
    "idle" | "running" | "done-ok" | "done-err"
  >("idle");
  const [receiveLog, setReceiveLog] = useState<string[]>([]);
  // 最近一次接收的目标目录（"打开接收目录"用）
  const [receiveDir, setReceiveDir] = useState<string | null>(null);
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

  // 挂载时加载历史；收发完成后刷新
  useEffect(() => {
    void loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [receiveStatus === "done-ok", share.status === "done"]);

  // 监听共享事件流（share://code / progress / done / error）
  useEffect(() => {
    const unCode = listen<string>("share://code", (e) => {
      setShare({ status: "code", code: e.payload });
      setShareLog((l) => [...l, `传输码: ${e.payload}`]);
    });
    const unProgress = listen<string>("share://progress", (e) => {
      setShareLog((l) => [...l.slice(-200), e.payload]);
    });
    const unDone = listen<number>("share://done", (e) => {
      setShare({ status: "done", ok: e.payload === 0 });
      if (e.payload !== 0) setShareLog((l) => [...l, "共享失败"]);
    });
    const unError = listen<string>("share://error", (e) => {
      setShare({ status: "done", ok: false });
      setShareLog((l) => [...l, `共享错误: ${e.payload}`]);
    });
    // 接收事件流（receive://progress / done / error）
    const unRProgress = listen<string>("receive://progress", (e) => {
      setReceiveLog((l) => [...l.slice(-200), e.payload]);
    });
    const unRDone = listen<number>("receive://done", (e) => {
      setReceiveStatus(e.payload === 0 ? "done-ok" : "done-err");
      if (e.payload !== 0) setReceiveLog((l) => [...l, "接收失败"]);
    });
    const unRError = listen<string>("receive://error", (e) => {
      setReceiveStatus("done-err");
      setReceiveLog((l) => [...l, `接收错误: ${e.payload}`]);
    });
    return () => {
      unCode.then((u) => u());
      unProgress.then((u) => u());
      unDone.then((u) => u());
      unError.then((u) => u());
      unRProgress.then((u) => u());
      unRDone.then((u) => u());
      unRError.then((u) => u());
    };
  }, []);

  async function startShare(path: string) {
    setShare({ status: "running" });
    setShareLog([]);
    try {
      await invoke("share_dataset", { path });
    } catch (e) {
      setShare({ status: "done", ok: false });
      setShareLog((l) => [...l, `共享启动失败: ${String(e)}`]);
    }
  }

  async function startReceive() {
    const code = receiveCode.trim();
    if (!code || receiveStatus === "running") return;
    setReceiveStatus("running");
    setReceiveLog([]);
    try {
      // 接收输出到 ~/Downloads/fan-received（默认接收目录）
      const home = await invoke<string>("fan_home");
      const downloads = home.replace("/.fan-files", "/Downloads/fan-received");
      setReceiveDir(downloads);
      await invoke("receive_dataset", { code, output: downloads });
    } catch (e) {
      setReceiveStatus("done-err");
      setReceiveLog((l) => [...l, `接收启动失败: ${String(e)}`]);
    }
  }

  function openReceiveDir() {
    if (receiveDir) {
      invoke("open_path", { path: receiveDir }).catch(console.error);
    }
  }

  // 游标分页：next_cursor 非空则"下一页"可用（cursor 即上一页最后一条的 id）。
  // 错误在内部消化（不向外 reject），失败时清空行/游标/历史栈。
  async function load(cursor?: number, selectedType?: string) {
    setLoading(true);
    try {
      const page = await fetchDatasets({ cursor, limit: 50, type: selectedType });
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

  useEffect(() => {
    setHistory([]); // 类型筛选变化时清空历史栈
    void load(undefined, type);
  }, [type]);

  function goNext() {
    if (loading || !nextCursor) return;
    setHistory((h) => [...h, nextCursor]);
    void load(nextCursor, type);
  }

  function goPrev() {
    if (loading || history.length === 0) return;
    // pop 栈顶（当前页的 cursor），load 新栈顶即上一页的 cursor；栈空为 undefined 回第一页
    const prevTop = history.length > 1 ? history[history.length - 2] : undefined;
    setHistory((h) => h.slice(0, -1));
    void load(prevTop, type);
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
        {receiveLog.length > 0 && (
          <pre className="receive-log">{receiveLog.join("\n")}</pre>
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
      {detail && (
        <div className="modal" onClick={() => setDetail(null)}>
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
                    <code className="share-code-value">{share.code}</code>
                    <div className="share-code-cmd">
                      fan-files transfer get {share.code}
                    </div>
                  </div>
                )}
                {share.status === "running" && <div className="share-hint">⏳ 正在连接 rendezvous…</div>}
                {share.status === "done" && (
                  <div className={share.ok ? "feedback-ok" : "feedback-err"}>
                    {share.ok ? "✅ 共享完成" : "❌ 共享失败"}
                  </div>
                )}
                {shareLog.length > 0 && (
                  <pre className="share-log">{shareLog.join("\n")}</pre>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
