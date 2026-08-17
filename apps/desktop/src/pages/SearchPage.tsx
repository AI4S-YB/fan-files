import { useState, type FormEvent } from "react";
import { searchDatasets, type DatasetSummary } from "../api";
import DataTable from "../components/DataTable";

export default function SearchPage() {
  const [q, setQ] = useState("");
  // rows === null 表示"尚未搜索"；[] 表示"搜索过但没有结果"
  const [rows, setRows] = useState<DatasetSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    // 客户端校验：q 为空不发起请求（后端空 q 会 400）
    if (!q.trim()) return;
    try {
      setError(null);
      setRows(await searchDatasets(q.trim()));
    } catch {
      // 失败时不清空已有结果，仅显示错误行（T15 全局横幅前的最小反馈）
      setError("搜索失败，请检查引擎状态");
    }
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
          // 详情弹层复用留待 T11.5/T12（抽公共组件后再接，避免复制 DatasetsPage 的 modal 逻辑）
          onSelect={() => {}}
          emptyText="没有找到匹配的数据集 — 试试换关键词（如：水稻基因组）"
        />
      )}
    </div>
  );
}
