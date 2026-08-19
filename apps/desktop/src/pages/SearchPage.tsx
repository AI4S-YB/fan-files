import { useRef, useState, type FormEvent } from "react";
import {
  searchDatasets,
  fetchDatasetDetail,
  fetchFiles,
  type DatasetSummary,
  type DatasetDetail,
  type FileSummary,
} from "../api";
import DataTable from "../components/DataTable";
import DatasetDetailModal from "../components/DatasetDetailModal";

export default function SearchPage() {
  const [q, setQ] = useState("");
  // rows === null 表示"尚未搜索"；[] 表示"搜索过但没有结果"
  const [rows, setRows] = useState<DatasetSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 请求序号（last-write-wins）：连发两次搜索时，旧响应返回后不覆盖新结果
  const seq = useRef(0);
  // GUI-T4: 结果详情弹层（复用 DatasetDetailModal）
  const [detail, setDetail] = useState<DatasetDetail | null>(null);
  const [files, setFiles] = useState<FileSummary[]>([]);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    // 客户端校验：q 为空不发起请求（后端空 q 会 400）
    if (!q.trim()) return;
    const id = ++seq.current;
    try {
      const result = await searchDatasets(q.trim());
      if (id !== seq.current) return; // 已有更新的请求，丢弃陈旧响应
      setError(null);
      setRows(result);
    } catch {
      if (id !== seq.current) return;
      // 失败时不清空已有结果，仅显示错误行（T15 全局横幅前的最小反馈）
      setError("搜索失败，请检查引擎状态");
    }
  }

  // 打开结果详情：与数据集页同构（详情失败静默返回；文件列表失败静默为空）
  async function openDetail(r: DatasetSummary) {
    setFiles([]);
    let d: DatasetDetail;
    try {
      d = await fetchDatasetDetail(r.id);
    } catch {
      return;
    }
    setDetail(d);
    fetchFiles(r.id)
      .then((page) => setFiles(page.data))
      .catch(() => setFiles([]));
  }

  return (
    <div className="page">
      <h2>搜索</h2>
      <form role="search" onSubmit={submit}>
        <input
          role="searchbox"
          className="search-box"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="搜索你的数据（如：水稻基因组）…"
        />
        <button type="submit" className="primary">搜索</button>
      </form>
      {error && <div className="search-error">{error}</div>}
      {rows === null ? (
        <div className="empty">输入关键词或自然语言描述，搜索你的数据集</div>
      ) : (
        <DataTable
          rows={rows}
          // GUI-T4: 结果行可点详情（复用 DatasetDetailModal，含共享按钮）
          onSelect={openDetail}
          emptyText="没有找到匹配的数据集 — 试试换关键词（如：水稻基因组）"
        />
      )}
      {detail && (
        <DatasetDetailModal detail={detail} files={files} onClose={() => setDetail(null)} />
      )}
    </div>
  );
}
