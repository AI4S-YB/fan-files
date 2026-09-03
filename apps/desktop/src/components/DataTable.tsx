import type { DatasetSummary } from "../api";

// 与 App.css 的 badge-* 类别色一一对应；未收录的 type 回退 badge-other 样式
const KNOWN_TYPES = new Set(["genome", "transcriptome", "variant", "other"]);

export default function DataTable({
  rows,
  onSelect,
  emptyText = "还没有数据集 — 去首页开始扫描",
}: {
  rows: DatasetSummary[];
  onSelect: (r: DatasetSummary) => void;
  // 空状态文案可定制（T11 搜索页将传自己的示例查询提示）
  emptyText?: string;
}) {
  if (rows.length === 0) {
    return <div className="empty">{emptyText}</div>;
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
        {rows.map((r) => {
          const badgeType = r.type && KNOWN_TYPES.has(r.type) ? r.type : "other";
          return (
            <tr key={r.id} onClick={() => onSelect(r)}>
              <td>{r.name}</td>
              <td>
                <span className={`badge badge-${badgeType}`}>{r.type ?? "—"}</span>
              </td>
              <td>{r.species ?? "—"}</td>
              <td>{r.file_count.toLocaleString()}</td>
              <td className="mono">{r.path ?? "—"}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
