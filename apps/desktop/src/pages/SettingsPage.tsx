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

// [transfer] 段（与后端 read_transfer_config 返回形状一致：chunk_size_mb/concurrency/
// receive_dir/udp_enabled；缺失键回退默认）
interface TransferCfg {
  chunk_size_mb: number;
  concurrency: number;
  receive_dir: string | null;
  udp_enabled: boolean;
}

const DEFAULT_TRANSFER: TransferCfg = {
  chunk_size_mb: 4,
  concurrency: 4,
  receive_dir: null,
  udp_enabled: true,
};

export default function SettingsPage() {
  const [cfg, setCfg] = useState<FanConfig>(EMPTY);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorLabel, setErrorLabel] = useState("保存失败");
  const [testResult, setTestResult] = useState<{ ok: boolean; ms: number } | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [testRunning, setTestRunning] = useState(false);
  const [updateText, setUpdateText] = useState<string | null>(null);
  // 传输参数（[transfer] 段，独立加载/保存）
  const [transfer, setTransfer] = useState<TransferCfg>(DEFAULT_TRANSFER);
  const [transferSaved, setTransferSaved] = useState(false);
  const [transferError, setTransferError] = useState<string | null>(null);

  useEffect(() => {
    invoke<FanConfig>("read_config")
      .then(setCfg)
      .catch(() => setCfg(EMPTY)); // 读取失败 → 全空默认（同 T9 模式）
  }, []);

  useEffect(() => {
    invoke<Partial<TransferCfg>>("read_transfer_config")
      .then((c) => setTransfer({ ...DEFAULT_TRANSFER, ...c }))
      .catch(() => setTransfer(DEFAULT_TRANSFER)); // 读取失败 → 默认值
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
        setErrorLabel("保存失败");
        setError(String(e));
      });
  };

  // T13: 原生目录选择器 pick_directory；取消（null）或重复目录不改变 include
  const addDirectory = async () => {
    try {
      const dir = await invoke<string | null>("pick_directory");
      if (!dir) return;
      setCfg((c) => (c.include.includes(dir) ? c : { ...c, include: [...c.include, dir] }));
      setSaved(false);
      setError(null);
    } catch (e) {
      setErrorLabel("添加目录失败");
      setError(String(e));
    }
  };

  // T13: 后端 test_connection(cfg: FanConfig) → 按参数名传 { cfg }
  const testConnection = async () => {
    setTestRunning(true);
    setTestResult(null);
    setTestError(null);
    const t0 = performance.now();
    try {
      const ok = await invoke<boolean>("test_connection", { cfg });
      setTestResult({ ok, ms: Math.round(performance.now() - t0) });
    } catch (e) {
      setTestResult({ ok: false, ms: Math.round(performance.now() - t0) });
      setTestError(String(e));
    } finally {
      setTestRunning(false);
    }
  };

  // T13: 后端 check_update 返回 CLI 输出文本，直接展示（T16 sidecar 定位前为 PATH 临时实现）
  const checkUpdate = async () => {
    setUpdateText(null);
    try {
      const out = await invoke<string>("check_update");
      setUpdateText(out);
    } catch (e) {
      setUpdateText(`检查更新失败：${String(e)}`);
    }
  };

  // ---- 传输参数（[transfer] 段）----

  const patchTransfer = <K extends keyof TransferCfg>(key: K, value: TransferCfg[K]) => {
    setTransfer((t) => ({ ...t, [key]: value }));
    setTransferSaved(false);
    setTransferError(null);
  };

  // 保存 [transfer] 段：read-modify-write 由后端保证；receive_dir=null 会删除该键
  const saveTransfer = () => {
    const cfg: TransferCfg = {
      chunk_size_mb: transfer.chunk_size_mb,
      concurrency: transfer.concurrency,
      receive_dir: transfer.receive_dir,
      udp_enabled: transfer.udp_enabled,
    };
    invoke("write_transfer_config", { cfg })
      .then(() => {
        setTransferSaved(true);
        setTransferError(null);
      })
      .catch((e) => {
        setTransferSaved(false);
        setTransferError(String(e));
      });
  };

  // 默认接收目录选择（复用原生目录选择器 pick_directory；取消（null）不修改）
  const pickReceiveDir = async () => {
    try {
      const dir = await invoke<string | null>("pick_directory");
      if (dir) {
        patchTransfer("receive_dir", dir);
      }
    } catch (e) {
      setTransferError(String(e));
    }
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
        <button className="secondary" onClick={addDirectory}>
          📁 添加目录
        </button>
      </section>

      {/* GUI-T3: 传输参数（config [transfer] 段，独立保存） */}
      <section className="settings-section">
        <h3>传输设置</h3>
        <label className="field" htmlFor="chunk-size">
          <span>块大小（MB）</span>
          <select
            id="chunk-size"
            value={transfer.chunk_size_mb}
            onChange={(e) => patchTransfer("chunk_size_mb", Number(e.target.value))}
          >
            {[2, 4, 8, 16].map((v) => (
              <option key={v} value={v}>
                {v} MB
              </option>
            ))}
          </select>
        </label>
        <label className="field" htmlFor="concurrency">
          <span>并发数（同时传输的块数）</span>
          <select
            id="concurrency"
            value={transfer.concurrency}
            onChange={(e) => patchTransfer("concurrency", Number(e.target.value))}
          >
            {[1, 2, 4, 8].map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        </label>
        <div className="field">
          <span>默认接收目录</span>
          <div className="dir-row">
            <input
              aria-label="默认接收目录"
              readOnly
              className="dir-input dir-readonly"
              value={
                transfer.receive_dir ?? "未设置（默认 ~/Downloads/fan-received）"
              }
            />
            <button className="secondary" onClick={pickReceiveDir}>
              📁 选择目录
            </button>
            {transfer.receive_dir && (
              <button
                className="secondary"
                onClick={() => patchTransfer("receive_dir", null)}
              >
                清除
              </button>
            )}
          </div>
        </div>
        <label className="field toggle-row" htmlFor="udp-enabled">
          <span>启用 UDP 直连（P2P 打洞，失败自动降级中继）</span>
          <input
            id="udp-enabled"
            type="checkbox"
            checked={transfer.udp_enabled}
            onChange={(e) => patchTransfer("udp_enabled", e.target.checked)}
          />
        </label>
        <div className="settings-actions">
          <button className="secondary" onClick={saveTransfer}>
            保存传输设置
          </button>
          {transferSaved && <span className="feedback-ok">已保存 ✓</span>}
          {transferError && (
            <span className="feedback-err">保存失败：{transferError}</span>
          )}
        </div>
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
        <button className="secondary" onClick={testConnection} disabled={testRunning}>
          {testRunning ? "测试中…" : "测试连接"}
        </button>
        {testResult &&
          (testResult.ok ? (
            <span className="feedback-ok">连接成功 ✓（{testResult.ms}ms）</span>
          ) : (
            <span className="feedback-err">
              连接失败（{testResult.ms}ms）{testError ? `：${testError}` : ""}
            </span>
          ))}
      </section>

      <section className="settings-section">
        <h3>账号与崖州湾试用</h3>
        <p className="settings-hint">账号管理与崖州湾试用功能即将上线，敬请期待。</p>
      </section>

      <section className="settings-section">
        <h3>关于</h3>
        {/* 版本与 package.json / src-tauri/tauri.conf.json 同步 */}
        <p className="settings-hint">fan-files desktop v0.1.0</p>
        <button className="secondary" onClick={checkUpdate}>
          检查更新
        </button>
        {updateText && <span className="settings-hint update-feedback">{updateText}</span>}
      </section>

      <div className="settings-actions">
        <button className="primary" onClick={save}>
          保存配置
        </button>
        {saved && <span className="feedback-ok">已保存 ✓</span>}
        {error && <span className="feedback-err">{errorLabel}：{error}</span>}
      </div>
    </div>
  );
}
