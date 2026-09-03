// SF-T4: CC Switch profile 选择弹窗。
// SettingsPage「从 CC Switch 接管」检测到多个 profile 时弹出，每个 profile 一行：
// 名称 + 协议徽标（Anthropic/OpenAI/未知）+ 模型名。
// 纯展示组件：选中/取消回调由父级注入（模式同 DatasetDetailModal），
// 关闭时不填充表单（取消语义由父级 onClose 保证）。

// CC Switch profile 摘要（fan-files config cc-switch --list 输出的元素形状）
export interface CcProfile {
  name: string;
  api_type: string; // "openai" | "anthropic" | ""（未识别协议，引擎 profile_summary 语义）
  model: string;
}

export default function ProfilePicker({
  profiles,
  onSelect,
  onClose,
}: {
  profiles: CcProfile[];
  onSelect: (name: string) => void;
  onClose: () => void;
}) {
  return (
    <div className="modal" onClick={onClose} role="dialog" aria-label="选择 CC Switch Profile">
      <div className="modal-body" onClick={(e) => e.stopPropagation()}>
        <h3>选择 CC Switch Profile</h3>
        <p className="settings-hint">检测到多个 CC Switch 配置，选择要接管的 profile：</p>
        <ul className="cc-profile-list">
          {profiles.map((p) => (
            <li key={p.name}>
              <button className="cc-profile-row" onClick={() => onSelect(p.name)}>
                <span className="cc-profile-name">{p.name}</span>
                <span className={`cc-profile-badge ${p.api_type}`}>
                  {p.api_type === "anthropic"
                    ? "Anthropic"
                    : p.api_type === "openai"
                      ? "OpenAI"
                      : "未知"}
                </span>
                <span className="cc-profile-model">{p.model || "—"}</span>
              </button>
            </li>
          ))}
        </ul>
        <div className="modal-actions">
          <button className="secondary" onClick={onClose}>
            取消
          </button>
        </div>
      </div>
    </div>
  );
}
