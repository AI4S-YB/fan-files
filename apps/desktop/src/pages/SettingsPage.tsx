import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// 与后端 T5 read_config 命令返回的形状一致（见 src-tauri FanConfig）。
// threads 不进 UI，但必须保留在接口以透传保存（否则 GUI 保存会把文件的 threads 键删掉）。
interface FanConfig {
  threads?: number | null;
  include: string[];
  exclude: string[];
  endpoint: string;
  api_key: string;
  model: string;
}

const EMPTY: FanConfig = { threads: null, include: [], exclude: [], endpoint: "", api_key: "", model: "" };

export default function SettingsPage() {
  const [cfg, setCfg] = useState<FanConfig>(EMPTY);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<FanConfig>("read_config")
      .then(setCfg)
      .catch(() => setCfg(EMPTY)); // 读取失败 → 全空默认（同 T9 模式）
  }, []);

  const patch = <K extends keyof FanConfig>(key: K, value: FanConfig[K]) => {
    setCfg((c) => ({ ...c, [key]: value }));
    setSaved(false);
    setError(null);
  };

  const patchInclude = (i: number, value: string) => {
    setCfg((c) => ({ ...c, include: c.include.map((d, idx) => (idx === i ? value : d)) }));
    setSaved(false);
    setError(null);
  };

  const removeInclude = (i: number) => {
    setCfg((c) => ({ ...c, include: c.include.filter((_, idx) => idx !== i) }));
    setSaved(false);
    setError(null);
  };

  const save = () => {
    // Tauri 2 按 Rust 参数名取键：write_config(cfg: FanConfig) 需要 args["cfg"]，
    // 扁平传参会报 "missing required key cfg"（tauri 2.11.5 源码确认）。
    invoke("write_config", { cfg })
      .then(() => {
        setSaved(true);
        setError(null);
      })
      .catch((e) => {
        setSaved(false);
        setError(String(e));
      });
  };

  return (
    <div className="page settings">
      <h2>设置</h2>

      <section className="settings-section">
        <h3>数据目录</h3>
        {cfg.include.length === 0 ? (
          <p className="settings-hint">还没有添加数据目录。扫描会遍历这些目录下的数据集。</p>
        ) : (
          <ul className="dir-list">
            {cfg.include.map((dir, i) => (
              <li key={i} className="dir-row">
                <input
                  aria-label={`目录 ${i + 1}`}
                  className="dir-input"
                  value={dir}
                  onChange={(e) => patchInclude(i, e.target.value)}
                />
                <button className="secondary" onClick={() => removeInclude(i)}>
                  移除
                </button>
              </li>
            ))}
          </ul>
        )}
        {/* TODO(T13): 打开原生目录选择器（dialog 命令） */}
        <button className="secondary">📁 添加目录</button>
      </section>

      <section className="settings-section">
        <h3>模型配置</h3>
        <label className="field" htmlFor="endpoint">
          <span>Endpoint</span>
          <input id="endpoint" value={cfg.endpoint} onChange={(e) => patch("endpoint", e.target.value)} />
        </label>
        <label className="field" htmlFor="api-key">
          <span>API Key</span>
          <input
            id="api-key"
            type="password"
            value={cfg.api_key}
            onChange={(e) => patch("api_key", e.target.value)}
          />
        </label>
        <label className="field" htmlFor="model">
          <span>模型名称</span>
          <input id="model" value={cfg.model} onChange={(e) => patch("model", e.target.value)} />
        </label>
        {/* TODO(T13): 调用后端 test_connection 命令并展示耗时/结果 */}
        <button className="secondary">测试连接</button>
      </section>

      <section className="settings-section">
        <h3>账号与崖州湾试用</h3>
        <p className="settings-hint">账号管理与崖州湾试用功能即将上线，敬请期待。</p>
      </section>

      <section className="settings-section">
        <h3>关于</h3>
        {/* 版本与 package.json / src-tauri/tauri.conf.json 同步 */}
        <p className="settings-hint">fan-files desktop v0.1.0</p>
        {/* TODO(T13): 检查新版本并提示更新 */}
        <button className="secondary">检查更新</button>
      </section>

      <div className="settings-actions">
        <button className="primary" onClick={save}>
          保存配置
        </button>
        {saved && <span className="feedback-ok">已保存 ✓</span>}
        {error && <span className="feedback-err">保存失败：{error}</span>}
      </div>
    </div>
  );
}
