// share 实际端口由后端（get_share_port）提供——冲突时可能回退，故模块级可变。
let base = "http://127.0.0.1:17951";

export function setApiBase(port: number) {
  base = `http://127.0.0.1:${port}`;
}

// 只读导出，供测试断言 base 已被更新（正常业务代码不应读它）。
export function getApiBase(): string {
  return base;
}

// 形状以 crates/fan-files-share/src/models.rs 为准（serde 序列化后的 JSON 键名）。

export interface Stats {
  datasets_upper_bound: number;
  assets_upper_bound: number;
  files_upper_bound: number;
  linked_files_upper_bound: number;
  last_indexed_at: number | null;
  approximate: boolean;
}

export interface Facet {
  value: string;
  count: number;
}

export interface PageMeta {
  limit: number;
  next_cursor: number | null;
  has_more: boolean;
  sort?: string;
  type_counts?: Facet[];
}

export interface PageEnvelope<T> {
  data: T[];
  meta: PageMeta;
}

export interface DatasetSummary {
  id: number;
  name: string;
  type: string | null;
  species: string | null;
  summary: string | null;
  path: string | null;
  asset_count: number;
  file_count: number;
  updated_at: number;
}

export interface AssetSummary {
  id: number;
  name: string | null;
  type: string | null;
  file_count: number;
}

export interface DatasetDetail {
  id: number;
  name: string;
  type: string | null;
  species: string | null;
  species_confidence: string | null;
  summary: string | null;
  path: string | null;
  updated_at: number;
  assets: AssetSummary[];
}

export interface FileSummary {
  id: number;
  asset_id: number;
  name: string;
  size: number;
  role: string | null;
  mime_type: string | null;
  source_server: string;
  path: string | null;
}

// 对话搜索（NR-T5）：messages 为多轮上下文，question 为当前问题
export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

// LLM 生成的结构化查询（响应中原样返回，前端可展开展示）
export interface ChatQuery {
  keywords: string[];
  type?: string | null;
}

export interface ChatSearchResp {
  query: ChatQuery;
  results: DatasetSummary[];
}

// 对应后端 DatasetQuery：q/species/type/cursor/limit/sort/order。
// sort：id 默认；relevance（需 q）；name/file_count（GUI-T4 数据集页排序下拉）。
// order 仅 "asc"（服务端校验；order 缺省也按 asc）。
export interface DatasetQuery {
  q?: string;
  species?: string;
  type?: string;
  cursor?: number;
  limit?: number;
  sort?: "id" | "relevance" | "name" | "file_count";
  order?: "asc";
}

function toParams(q: DatasetQuery): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(q)) {
    if (value !== undefined && value !== null) {
      params.set(key, String(value));
    }
  }
  return params.toString();
}

// HTTP 非 2xx 时抛出，带 status 供调用方区分：
// - 带 status 的 ApiError → 引擎在线但返回错误（chat-search 503 = LLM 未配置/失败）
// - fetch 网络错误（引擎未启动/连接拒绝）→ 原生 TypeError，无 status
export class ApiError extends Error {
  readonly status: number;
  constructor(status: number) {
    super(`HTTP ${status}`);
    this.name = "ApiError";
    this.status = status;
  }
}

// Envelope 包装：{ data: T }
async function get<T>(path: string): Promise<T> {
  const r = await fetch(`${base}${path}`);
  if (!r.ok) throw new ApiError(r.status);
  const body = await r.json();
  return body.data as T;
}

// 分页 Envelope：{ data: T[], meta: PageMeta }
async function getPage<T>(path: string): Promise<PageEnvelope<T>> {
  const r = await fetch(`${base}${path}`);
  if (!r.ok) throw new ApiError(r.status);
  return (await r.json()) as PageEnvelope<T>;
}

// POST + Envelope 包装：{ data: T }（chat-search 等写/语义端点）
async function post<T>(path: string, body: unknown): Promise<T> {
  const r = await fetch(`${base}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new ApiError(r.status);
  const data = await r.json();
  return data.data as T;
}

export const fetchStats = () => get<Stats>("/api/v1/stats");
export const fetchDatasets = (q: DatasetQuery = {}) => {
  const params = toParams(q);
  return getPage<DatasetSummary>(`/api/v1/datasets${params ? `?${params}` : ""}`);
};
export const searchDatasets = (q: string) =>
  get<DatasetSummary[]>(`/api/v1/search?q=${encodeURIComponent(q)}`);
// 对话搜索（NR-T5）：多轮上下文 + 当前问题 → LLM 结构化查询 + 结果。
// LLM 未配置/失败 → 503（code llm_unavailable），调用方降级基础搜索。
export const chatSearch = (messages: ChatMessage[], question: string) =>
  post<ChatSearchResp>("/api/v1/chat-search", { messages, question });
export const fetchDatasetDetail = (id: number) => get<DatasetDetail>(`/api/v1/datasets/${id}`);
// 文件分页（详情弹层只取第一页；FileQuery: asset_id/cursor/limit）
export const fetchFiles = (id: number, cursor?: number) =>
  getPage<FileSummary>(
    `/api/v1/datasets/${id}/files?limit=50${cursor !== undefined ? `&cursor=${cursor}` : ""}`
  );
