import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Send, Sparkles, Search as SearchIcon, AlertCircle, RefreshCw, ChevronRight, Brain, X } from "lucide-react";
import {
  searchDatasets,
  chatSearch,
  fetchDatasetDetail,
  fetchFiles,
  fetchDatasets,
  fetchStats,
  ApiTimeoutError,
  type DatasetSummary,
  type DatasetDetail,
  type FileSummary,
  type ChatQuery,
} from "../api";
import DataTable from "../components/DataTable";
import DatasetDetailModal from "../components/DatasetDetailModal";
import SharePanel from "../components/SharePanel";
import ResumeDialog from "../components/ResumeDialog";
import { useShareTransfer } from "../hooks/useShareTransfer";

// read_config 返回形状（与 SettingsPage 的 FanConfig 同构）；只看 api_key 是否配置
interface FanConfig {
  api_key: string;
  [k: string]: unknown;
}

// 对话回合：user = 提问；assistant = 结果摘要（含 LLM 查询与结果表格）
export interface ChatTurn {
  role: "user" | "assistant";
  content: string; // 问题（user）/ 摘要（assistant）
  query?: ChatQuery; // assistant：LLM 生成的结构化查询（可展开）
  results?: DatasetSummary[]; // assistant：搜索结果
  fallback?: boolean; // assistant：LLM 失败降级基础搜索
}

// 快捷示例问题（空态时显示）；"查看所有" 走专用路径，其他走 AI 搜索
const EXAMPLE_QUESTIONS = [
  { q: "我想要水稻参考基因组",     label: "🐬 水稻参考基因组" },
  { q: "找近半年更新的转录组数据",  label: "📡 近半年转录组" },
  { q: "有没有水稻变异数据集？",   label: "🔬 水稻变异数据" },
] as const;

interface SearchPageProps {
  turns: ChatTurn[];
  setTurns: React.Dispatch<React.SetStateAction<ChatTurn[]>>;
}

export default function SearchPage({ turns, setTurns }: SearchPageProps) {
  // NR-T5: 挂载时读一次 config 判断 LLM 是否配置（api_key 非空）。
  // true → 对话模式（多轮）；false → 基础搜索（单次）+ 提示。
  // NR-T5: 初始 false（同步渲染基础模式，避免测试用 fake timers）。
  // config 异步到达后：api_key 非空 → 升到 chat mode；空/失败 → 保持基础模式。
  // 此设计等价于原逻辑（false 直接渲染），但带异步升级能力。
  const [llmConfigured, setLlmConfigured] = useState<boolean>(false);
  // 对话模式状态（turns 由 App 提升上去，切走页面后会话保持）
  const [chatInput, setChatInput] = useState("");
  const [chatPending, setChatPending] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);
  // 基础搜索模式状态（无模型 / LLM 失败降级后仍可继续使用）
  const [q, setQ] = useState("");
  // rows === null 表示"尚未搜索"；[] 表示"搜索过但没有结果"
  const [rows, setRows] = useState<DatasetSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // SF-T3: 扫描完成（fan-scan-done）→ 旧结果可能过期，清空并提示重新搜索
  const [scanUpdated, setScanUpdated] = useState(false);
  // 请求序号（last-write-wins）：连发两次搜索时，旧响应返回后不覆盖新结果
  const seq = useRef(0);
  // NR-T5: 共享有效期（小时），默认 168 = 引擎默认 7 天；弹层选择后随共享传递
  const [ttlHours, setTtlHours] = useState(168);
  // 结果详情弹层（复用 DatasetDetailModal）
  const [detail, setDetail] = useState<DatasetDetail | null>(null);
  const [files, setFiles] = useState<FileSummary[]>([]);
  // 滚动到底部用的 ref
  const messagesEndRef = useRef<HTMLDivElement>(null);
  // 输入框 ref（自动 grow）
  const inputRef = useRef<HTMLTextAreaElement>(null);
  // GUI-T5: 共享状态提升到页面级（弹层关闭后传输仍被跟踪），与数据集页同构
  const {
    share,
    shareTtl,
    shareEvents,
    shareRaw,
    shareName,
    shareResume,
    startShare,
    cancelShare,
    continueResume,
    rejectResume,
  } = useShareTransfer();

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const cfg = await invoke<FanConfig | undefined>("read_config");
        if (!cancelled) setLlmConfigured(Boolean(cfg?.api_key));
      } catch {
        if (!cancelled) setLlmConfigured(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // SF-T3: 扫描完成（App 广播 fan-scan-done）→ 不清空历史会话（用户翻看时希望保留），
  // 只清基础搜索结果（rows），并提示用户重新搜索/提问
  useEffect(() => {
    const onScanDone = () => {
      setRows(null);
      setError(null);
      setChatError(null);
      setScanUpdated(true);
    };
    window.addEventListener("fan-scan-done", onScanDone);
    return () => window.removeEventListener("fan-scan-done", onScanDone);
  }, []);

  // 新消息追加时滚动到底部
  useEffect(() => {
    if (llmConfigured) {
      const el = messagesEndRef.current;
      if (el && typeof el.scrollIntoView === "function") {
        el.scrollIntoView({ behavior: "smooth", block: "end" });
      }
    }
  }, [turns, chatPending, llmConfigured]);

  // 基础搜索（无模型 / 降级后可用）
  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!q.trim()) return;
    setScanUpdated(false);
    const id = ++seq.current;
    try {
      const result = await searchDatasets(q.trim());
      if (id !== seq.current) return;
      setError(null);
      setRows(result);
    } catch {
      if (id !== seq.current) return;
      setError("搜索失败，请检查引擎状态");
    }
  }

  // 对话提问：LLM 生成查询 → 结果；LLM 层失败（HTTP 错误，如 503）→ 降级基础搜索 + 提示；
  // 引擎不可达（fetch 网络错误，无 status）→ 直接报错，不做无意义的降级。
  // 多轮：历史消息（turns）随 messages 传递，当前问题走 question 参数。
  async function ask(e?: FormEvent<HTMLFormElement>, presetQuestion?: string) {
    e?.preventDefault();
    const question = (presetQuestion ?? chatInput).trim();
    if (!question || chatPending) return;
    setScanUpdated(false);
    setTurns((t) => [...t, { role: "user", content: question }]);
    setChatInput("");
    setChatPending(true);
    setChatError(null);
    const history = turns.map((t) => ({ role: t.role, content: t.content }));
    try {
      const resp = await chatSearch(history, question);
      // 无关问题（问候/天气/闲聊）→ 友好提示，不走降级逻辑
      if (resp.reason === "irrelevant") {
        setTurns((t) => [
          ...t,
          {
            role: "assistant",
            content:
              "这个问题看起来和数据集搜索无关哦。我擅长帮你找数据 — 试试描述数据类型或物种名称，比如“水稻基因组”、“近半年转录组数据”。",
          },
        ]);
      } else {
        setTurns((t) => [
          ...t,
          {
            role: "assistant",
            content: `找到 ${resp.results.length} 个数据集`,
            query: resp.query,
            results: resp.results,
          },
        ]);
      }
    } catch (err: unknown) {
      if (err instanceof ApiTimeoutError) {
        // 接口卡死（share 进程/引擎繁忙）→ 友好降级，不留悬念的转圈
        setTurns((t) => [
          ...t,
          {
            role: "assistant",
            content: `请求超时（>${err.ms}ms），引擎可能繁忙 —— 稍后再试，或点击「重新扫描」后刷新。`,
            results: [],
            fallback: true,
          },
        ]);
        // 超时后仍尝试一次基础搜索，万一引擎刚好好了
        try {
          const fb = await searchDatasets(question);
          if (fb.length > 0) {
            setTurns((t) => {
              const last = t[t.length - 1];
              if (last?.role !== "assistant" || !last.fallback) return t;
              return [
                ...t.slice(0, -1),
                { ...last, content: `找到 ${fb.length} 个数据集`, results: fb },
              ];
            });
          }
        } catch {
          // ignore
        }
      } else if (
        typeof (err as { status?: unknown } | null)?.status === "number"
      ) {
        // LLM 失败（HTTP 503 等）→ 降级基础搜索（本机搜索仍可用）；并把"查看全部"建议带回，
        // 让用户在 AI 不可用时也能快速浏览数据集。
        try {
          const results = await searchDatasets(question);
          setTurns((t) => [
            ...t,
            {
              role: "assistant",
              content: results.length > 0
                ? `找到 ${results.length} 个数据集`
                : `未找到匹配"${question}"的数据集`,
              results,
              fallback: true,
            },
          ]);
        } catch {
          setChatError("搜索失败，请检查引擎状态");
        }
      } else {
        setChatError("搜索失败，请检查引擎状态");
      }
    } finally {
      setChatPending(false);
      // 让 textarea 收缩回单行
      if (inputRef.current) inputRef.current.style.height = "auto";
    }
  }

  // 输入框 Enter 发送（Shift+Enter 换行）
  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void ask();
    }
  }

  // 自动 grow
  function autoGrow(e: KeyboardEvent<HTMLTextAreaElement>) {
    const el = e.currentTarget;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 120) + "px";
  }

  // 打开结果详情
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

  // "查看所有数据集"快捷动作（AI 降级后或问"有什么数据"时使用）
  async function viewAllDatasets() {
    setChatPending(true);
    setChatError(null);
    try {
      // 并行拿分页 + 总数
      const [page, stats] = await Promise.all([
        fetchDatasets({ limit: 50 }),
        fetchStats(),
      ]);
      const total = stats?.datasets_upper_bound ?? page.data.length;
      setTurns((t) => [
        ...t,
        {
          role: "assistant",
          content: total > 0
            ? `已索引 ${total} 个数据集（显示前 ${page.data.length} 条，去数据集页可翻页/筛选）`
            : `尚未索引到数据集，请先在设置页添加数据目录并扫描`,
          results: page.data,
          fallback: true,
        },
      ]);
    } catch {
      setChatError("搜索失败，请检查引擎状态");
    } finally {
      setChatPending(false);
    }
  }

  return (
    <div className="search-page">
      {llmConfigured ? (
        /* ===== AI 对话模式 ===== */
        <>
          {scanUpdated && (
            <div className="search-banner">
              <AlertCircle size={13} /> 数据已更新，请重新搜索
            </div>
          )}

          <div className="search-messages">
            {turns.length > 0 && (
              <div className="search-messages-header">
                <span className="search-messages-count">
                  {turns.filter((t) => t.role === "assistant").length} 条结果
                </span>
                <button
                  className="btn btn-ghost btn-sm"
                  onClick={() => { setTurns([]); setChatError(null); }}
                  title="清空会话"
                >
                  <X size={13} />
                  清空会话
                </button>
              </div>
            )}
            {turns.length === 0 ? (
              <div className="search-empty">
                <div className="search-empty-icon">
                  <Brain size={28} />
                </div>
                <h2>你好，我是 fan-files 检索助手</h2>
                <p>用自然语言描述你的需求，AI 会帮你找到匹配的数据集。可以多轮追问，逐步缩小范围。</p>
                <div className="search-examples">
                  {EXAMPLE_QUESTIONS.map((item, i) => (
                    <button
                      key={i}
                      className="search-example-chip"
                      onClick={() => ask(undefined, item.q)}
                      disabled={chatPending}
                    >
                      <span>{item.label}</span>
                      <ChevronRight size={12} />
                    </button>
                  ))}
                  <button
                    className="search-example-chip"
                    onClick={() => void viewAllDatasets()}
                    disabled={chatPending}
                  >
                    <SearchIcon size={11} />
                    <span>查看所有数据集</span>
                    <ChevronRight size={12} />
                  </button>
                </div>
              </div>
            ) : (
              <div className="search-conversation">
                {turns.map((t, i) => (
                  <div key={i} className={`search-bubble-row search-bubble-${t.role}`}>
                    {t.role === "assistant" && (
                      <div className="search-avatar search-avatar-ai"><Sparkles size={13} /></div>
                    )}
                    <div className="search-bubble">
                      <div className="search-bubble-text">{t.content}</div>
                      {t.role === "assistant" && t.fallback && (
                        <div className="search-bubble-meta">
                          <strong>基础模式</strong>
                          <span className="search-bubble-meta-sep">·</span>
                          <span>AI 检索暂不可用，已切换到本机搜索</span>
                          {t.results && t.results.length === 0 && (
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              style={{ marginLeft: 8 }}
                              onClick={() => void viewAllDatasets()}
                              disabled={chatPending}
                            >
                              查看所有数据集
                            </button>
                          )}
                        </div>
                      )}
                      {t.role === "assistant" && t.query && (
                        <details className="search-query-details">
                          <summary>查询详情</summary>
                          <div>
                            关键词：{t.query.keywords.join("、")}
                            {t.query.type ? ` · 类型：${t.query.type}` : ""}
                          </div>
                        </details>
                      )}
                      {t.role === "assistant" && t.results && (
                        <DataTable
                          rows={t.results}
                          onSelect={openDetail}
                          emptyText="没有找到匹配的数据集 — 试试换种说法"
                        />
                      )}
                    </div>
                    {t.role === "user" && (
                      <div className="search-avatar search-avatar-user"><SearchIcon size={13} /></div>
                    )}
                  </div>
                ))}
                {chatPending && (
                  <div className="search-bubble-row search-bubble-assistant">
                    <div className="search-avatar search-avatar-ai"><Sparkles size={13} /></div>
                    <div className="search-bubble">
                      <div className="search-typing">
                        <span className="search-typing-dot" />
                        <span className="search-typing-dot" />
                        <span className="search-typing-dot" />
                      </div>
                    </div>
                  </div>
                )}
                <div ref={messagesEndRef} />
              </div>
            )}
            {chatError && <div className="search-error">{chatError}</div>}
          </div>

          <form className="search-composer" onSubmit={(e) => void ask(e)}>
            <textarea
              ref={inputRef}
              className="search-composer-input"
              rows={1}
              value={chatInput}
              onChange={(e) => setChatInput(e.target.value)}
              onKeyDown={(e) => { onKey(e); autoGrow(e); }}
              placeholder="用自然语言描述你的需求，可多轮追问…"
              disabled={chatPending}
            />
            <button
              type="submit"
              className="search-composer-send"
              disabled={chatPending || !chatInput.trim()}
              aria-label="发送"
            >
              {chatPending ? <RefreshCw size={14} className="spin" /> : <Send size={14} />}
            </button>
          </form>
        </>
      ) : (
        /* ===== 基础搜索模式（无模型） ===== */
        <>
          {scanUpdated && (
            <div className="search-banner">
              <AlertCircle size={13} /> 数据已更新，请重新搜索
            </div>
          )}

          <div className="search-basic">
            <div className="search-basic-header">
              <div className="search-basic-hint">未配置模型，使用基础搜索</div>
            </div>
            <form className="search-composer" onSubmit={submit}>
              <textarea
                className="search-composer-input"
                rows={1}
                value={q}
                onChange={(e) => setQ(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(e as unknown as FormEvent<HTMLFormElement>); } }}
                placeholder="搜索你的数据（如：水稻基因组）…"
              />
              <button
                type="submit"
                className="search-composer-send"
                disabled={!q.trim()}
                aria-label="搜索"
              >
                <Send size={14} />
              </button>
            </form>
            {error && <div className="search-error">{error}</div>}
            <div className="search-basic-results">
              {rows === null ? (
                <div className="search-empty">
                  <div className="search-empty-icon">
                    <SearchIcon size={26} />
                  </div>
                  <p>输入关键词或自然语言描述，搜索你的数据集</p>
                </div>
              ) : (
                <DataTable
                  rows={rows}
                  onSelect={openDetail}
                  emptyText="没有找到匹配的数据集 — 试试换关键词（如：水稻基因组）"
                />
              )}
            </div>
          </div>
        </>
      )}

      {/* 页面级共享面板 */}
      {share.status !== "idle" && !detail && (
        <SharePanel
          name={shareName}
          code={share.status === "code" ? share.code : undefined}
          events={shareEvents}
          log={shareRaw}
          onCancel={() => void cancelShare()}
          ttlHours={shareTtl}
        />
      )}
      {detail && (
        <DatasetDetailModal
          detail={detail}
          files={files}
          onClose={() => setDetail(null)}
          share={share}
          shareEvents={shareEvents}
          shareRaw={shareRaw}
          shareName={shareName}
          ttlHours={ttlHours}
          onTtlChange={setTtlHours}
          onShareStart={(path) => void startShare(path, detail.name, ttlHours)}
          onShareCancel={() => void cancelShare()}
        />
      )}
      {shareResume && (
        <ResumeDialog
          done={shareResume.done}
          total={shareResume.total}
          onContinue={continueResume}
          onReject={rejectResume}
        />
      )}
    </div>
  );
}
