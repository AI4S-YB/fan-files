import { useEffect, useRef, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  searchDatasets,
  chatSearch,
  fetchDatasetDetail,
  fetchFiles,
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
interface ChatTurn {
  role: "user" | "assistant";
  content: string; // 问题（user）/ 摘要（assistant）
  query?: ChatQuery; // assistant：LLM 生成的结构化查询（可展开）
  results?: DatasetSummary[]; // assistant：搜索结果
  fallback?: boolean; // assistant：LLM 失败降级基础搜索
}

export default function SearchPage() {
  // NR-T5: 挂载时读一次 config 判断 LLM 是否配置（api_key 非空）。
  // true → 对话模式（多轮）；false → 基础搜索（单次）+ 提示。
  const [llmConfigured, setLlmConfigured] = useState<boolean | null>(null);
  // 对话模式状态
  const [turns, setTurns] = useState<ChatTurn[]>([]);
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
        const cfg = await invoke<FanConfig>("read_config");
        if (!cancelled) setLlmConfigured(Boolean(cfg && cfg.api_key));
      } catch {
        if (!cancelled) setLlmConfigured(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // SF-T3: 扫描完成（App 广播 fan-scan-done）→ 清空结果与对话（结果可能过期），
  // 提示用户重新搜索/提问
  useEffect(() => {
    const onScanDone = () => {
      setRows(null);
      setError(null);
      setTurns([]);
      setChatError(null);
      setScanUpdated(true);
    };
    window.addEventListener("fan-scan-done", onScanDone);
    return () => window.removeEventListener("fan-scan-done", onScanDone);
  }, []);

  // 基础搜索（无模型 / 降级后可用）
  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    // 客户端校验：q 为空不发起请求（后端空 q 会 400）
    if (!q.trim()) return;
    setScanUpdated(false);
    const id = ++seq.current;
    try {
      const result = await searchDatasets(q.trim());
      if (id !== seq.current) return; // 已有更新的请求，丢弃陈旧响应
      setError(null);
      setRows(result);
    } catch {
      if (id !== seq.current) return;
      // 失败时不清空已有结果，仅显示错误行
      setError("搜索失败，请检查引擎状态");
    }
  }

  // 对话提问：LLM 生成查询 → 结果；LLM 层失败（HTTP 错误，如 503）→ 降级基础搜索 + 提示；
  // 引擎不可达（fetch 网络错误，无 status）→ 直接报错，不做无意义的降级。
  // 多轮：历史消息（turns）随 messages 传递，当前问题走 question 参数。
  async function ask(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const question = chatInput.trim();
    if (!question || chatPending) return;
    setScanUpdated(false);
    setTurns((t) => [...t, { role: "user", content: question }]);
    setChatInput("");
    setChatPending(true);
    setChatError(null);
    const history = turns.map((t) => ({ role: t.role, content: t.content }));
    try {
      const resp = await chatSearch(history, question);
      setTurns((t) => [
        ...t,
        {
          role: "assistant",
          content: `找到 ${resp.results.length} 个数据集`,
          query: resp.query,
          results: resp.results,
        },
      ]);
    } catch (err) {
      // SF-T3 修复: 用 error.status 区分"引擎返回的 HTTP 错误"（LLM 层失败 → 降级）
      // 与"引擎不可达"（fetch 网络错误是原生 TypeError，无 status → 直接报错）
      if (typeof (err as { status?: unknown } | null)?.status === "number") {
        // LLM 失败 → 降级基础搜索（本机搜索仍可用）；降级提示随该回合展示
        try {
          const results = await searchDatasets(question);
          setTurns((t) => [
            ...t,
            { role: "assistant", content: `找到 ${results.length} 个数据集`, results, fallback: true },
          ]);
        } catch {
          setChatError("搜索失败，请检查引擎状态");
        }
      } else {
        setChatError("搜索失败，请检查引擎状态");
      }
    } finally {
      setChatPending(false);
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
      {llmConfigured ? (
        /* NR-T5: 对话模式（有模型）：消息气泡列表 + 输入框，可多轮追问 */
        <>
          {scanUpdated && <div className="search-hint">数据已更新，请重新搜索</div>}
          <div className="chat-list">
            {turns.length === 0 && (
              <div className="empty">
                输入自然语言描述你的需求，AI 帮你找数据集（可多轮追问）
              </div>
            )}
            {turns.map((t, i) => (
              <div key={i} className={`chat-turn chat-${t.role}`}>
                <div className="chat-bubble">{t.content}</div>
                {t.role === "assistant" && t.results && (
                  <div className="chat-results">
                    {t.query && (
                      <details className="llm-query">
                        <summary>LLM 查询</summary>
                        <div>
                          关键词: {t.query.keywords.join("、")}
                          {t.query.type ? ` · 类型: ${t.query.type}` : ""}
                        </div>
                      </details>
                    )}
                    {t.fallback && (
                      <div className="search-hint">模型调用失败，已切换基础搜索</div>
                    )}
                    <DataTable
                      rows={t.results}
                      onSelect={openDetail}
                      emptyText="没有找到匹配的数据集 — 试试换种说法"
                    />
                  </div>
                )}
              </div>
            ))}
            {chatPending && <div className="chat-pending">思考中…</div>}
          </div>
          {chatError && <div className="search-error">{chatError}</div>}
          <form role="search" onSubmit={ask}>
            <input
              role="searchbox"
              className="search-box"
              value={chatInput}
              onChange={(e) => setChatInput(e.target.value)}
              placeholder="用自然语言描述你的需求，可多轮追问…"
              disabled={chatPending}
            />
            <button type="submit" className="primary" disabled={chatPending}>
              发送
            </button>
          </form>
        </>
      ) : (
        /* 基础搜索（无模型配置）：单次搜索 + 提示 */
        <>
          <div className="search-hint">未配置模型，使用基础搜索</div>
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
          {scanUpdated && <div className="search-hint">数据已更新，请重新搜索</div>}
          {error && <div className="search-error">{error}</div>}
          {rows === null ? (
            <div className="empty">输入关键词或自然语言描述，搜索你的数据集</div>
          ) : (
            <DataTable
              rows={rows}
              onSelect={openDetail}
              emptyText="没有找到匹配的数据集 — 试试换关键词（如：水稻基因组）"
            />
          )}
        </>
      )}
      {/* 页面级共享面板（弹层关闭后传输仍可跟踪/取消；弹层打开时面板在弹层内展示） */}
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
      {/* 共享侧续传确认弹窗（share://progress resume 事件触发；继续=关弹窗，引擎已自动续传） */}
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
