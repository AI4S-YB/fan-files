import { invoke } from "@tauri-apps/api/core";
import type { DatasetDetail, FileSummary } from "../api";
import type { ShareState } from "../hooks/useShareTransfer";
import SharePanel from "./SharePanel";
import type { TransferEvent } from "./TransferPanel";

// GUI-T5 修复：共享状态与 share:// 监听从本组件提升到页面级（useShareTransfer），
// 弹层改为纯展示：详情/资产/文件 + 共享/打开目录按钮 + 由 props 注入的共享面板。
// 弹层关闭不再导致监听丢失——传输继续由页面级面板跟踪（进度/取消入口保留）。
export default function DatasetDetailModal({
  detail,
  files,
  onClose,
  share,
  shareEvents,
  shareRaw,
  shareName,
  onShareStart,
  onShareCancel,
}: {
  detail: DatasetDetail;
  files: FileSummary[];
  onClose: () => void;
  share: ShareState;
  shareEvents: TransferEvent[];
  shareRaw: string[];
  shareName: string;
  onShareStart: (path: string) => void;
  onShareCancel: () => void;
}) {
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
            onClick={() => detail.path && onShareStart(detail.path)}
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
        {/* 共享面板（配对码 + 传输面板）：状态来自页面级 useShareTransfer */}
        {share.status !== "idle" && (
          <SharePanel
            name={shareName || detail.name}
            code={share.status === "code" ? share.code : undefined}
            events={shareEvents}
            log={shareRaw}
            onCancel={onShareCancel}
          />
        )}
      </div>
    </div>
  );
}
