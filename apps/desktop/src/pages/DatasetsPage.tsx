import { useEffect, useState } from "react";
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
      <div className="filters">
        {chips.map((t) => (
          <button
            key={t}
            className={type === t ? "chip active" : "chip"}
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
              <button disabled title="即将推出">
                📤 共享
              </button>
              {/* Task 13 接 invoke("open_path", { path: detail.path })，本任务空实现 */}
              <button onClick={() => {}}>📂 打开目录</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
