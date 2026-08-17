import type { DatasetSummary } from "../api";

export default function DataTable({
  rows,
  onSelect,
}: {
  rows: DatasetSummary[];
  onSelect: (r: DatasetSummary) => void;
}) {
  if (rows.length === 0) {
    return <div className="empty">还没有数据集 — 去首页开始扫描</div>;
  }
  return (
    <table className="data-table">
      <thead>
        <tr>
          <th>名称</th>
          <th>类型</th>
          <th>物种</th>
          <th>文件</th>
          <th>路径</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id} onClick={() => onSelect(r)}>
            <td>{r.name}</td>
            <td>
              <span className={`badge badge-${r.type ?? "other"}`}>{r.type ?? "—"}</span>
            </td>
            <td>{r.species ?? "—"}</td>
            <td>{r.file_count.toLocaleString()}</td>
            <td className="mono">{r.path ?? "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
