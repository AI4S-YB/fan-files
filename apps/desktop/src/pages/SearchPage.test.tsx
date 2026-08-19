import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SearchPage from "./SearchPage";
import * as api from "../api";

vi.mock("../api");
// GUI-T4: 详情弹层复用 DatasetDetailModal —— GUI-T5 后共享状态由页面级 useShareTransfer
// 自持，测试通过 eventMock.emit 注入 share:// 事件驱动共享面板。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";

const eventMock = vi.hoisted(() => {
  const listeners = new Map<string, ((e: { payload: unknown }) => void)[]>();
  return {
    listeners,
    listen: (event: string, cb: (e: { payload: unknown }) => void) => {
      const arr = listeners.get(event) ?? [];
      arr.push(cb);
      listeners.set(event, arr);
      return Promise.resolve(() => undefined);
    },
    emit: (event: string, payload: unknown) => {
      for (const cb of listeners.get(event) ?? []) cb({ payload });
    },
    clear: () => listeners.clear(),
  };
});
vi.mock("@tauri-apps/api/event", () => ({ listen: eventMock.listen }));
const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.resetAllMocks();
  eventMock.clear();
});

describe("SearchPage", () => {
  it("shows hint before any search", () => {
    render(<SearchPage />);
    expect(screen.getByText(/输入关键词或自然语言描述/)).toBeInTheDocument();
  });
  it("does not call api on empty query", async () => {
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "   " } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    expect(mockedApi.searchDatasets).not.toHaveBeenCalled();
  });
  it("renders results and empty text after search", async () => {
    mockedApi.searchDatasets.mockResolvedValue([{ id: 1, name: "Oryza_sativa_v1", type: "genome", species: "Oryza sativa", path: "/a/v1", file_count: 3, asset_count: 2, summary: null, updated_at: 1787000000 }]);
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "水稻" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    await waitFor(() => expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument());
  });
  it("shows no-result text when api returns empty", async () => {
    mockedApi.searchDatasets.mockResolvedValue([]);
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "xyz" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    await waitFor(() => expect(screen.getByText(/没有找到匹配的数据集/)).toBeInTheDocument());
  });
  it("shows error line on api failure and keeps old results", async () => {
    mockedApi.searchDatasets.mockResolvedValueOnce([{ id: 1, name: "A", type: "genome", species: null, path: "/a", file_count: 1, asset_count: 1, summary: null, updated_at: 0 }]);
    mockedApi.searchDatasets.mockRejectedValueOnce(new Error("HTTP 500"));
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "a" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    await waitFor(() => expect(screen.getByText("A")).toBeInTheDocument());
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "b" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    await waitFor(() => expect(screen.getByText(/搜索失败/)).toBeInTheDocument());
    expect(screen.getByText("A")).toBeInTheDocument();
  });
  it("clears error line and shows results after a successful retry", async () => {
    mockedApi.searchDatasets
      .mockRejectedValueOnce(new Error("HTTP 500"))
      .mockResolvedValueOnce([{ id: 1, name: "Oryza_sativa_v1", type: "genome", species: "Oryza sativa", path: "/a/v1", file_count: 3, asset_count: 2, summary: null, updated_at: 1787000000 }]);
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "水稻" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    await waitFor(() => expect(screen.getByText(/搜索失败/)).toBeInTheDocument());
    // 重试同一查询：成功后错误行消失，新结果渲染
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    await waitFor(() => expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument());
    expect(screen.queryByText(/搜索失败/)).not.toBeInTheDocument();
  });
  it("ignores stale response when a newer search resolves first", async () => {
    let resolveFirst!: (r: api.DatasetSummary[]) => void;
    let resolveSecond!: (r: api.DatasetSummary[]) => void;
    mockedApi.searchDatasets
      .mockReturnValueOnce(new Promise((r) => (resolveFirst = r)) as never)
      .mockReturnValueOnce(new Promise((r) => (resolveSecond = r)) as never);
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "a" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "b" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    // 第二次请求先返回
    await act(async () => {
      resolveSecond([{ id: 2, name: "B_result", type: "genome", species: null, path: "/b", file_count: 1, asset_count: 1, summary: null, updated_at: 0 }]);
    });
    expect(screen.getByText("B_result")).toBeInTheDocument();
    // 第一次（陈旧）响应后返回，不得覆盖新结果
    await act(async () => {
      resolveFirst([{ id: 1, name: "A_result", type: "genome", species: null, path: "/a", file_count: 1, asset_count: 1, summary: null, updated_at: 0 }]);
    });
    expect(screen.getByText("B_result")).toBeInTheDocument();
    expect(screen.queryByText("A_result")).not.toBeInTheDocument();
  });
  // GUI-T4: 结果行点击 → 打开 DatasetDetailModal（详情 + 资产 + 文件 + 共享按钮）
  it("opens the dataset detail modal on result click", async () => {
    mockedApi.searchDatasets.mockResolvedValue([{ id: 1, name: "Oryza_sativa_v1", type: "genome", species: "Oryza sativa", path: "/a/v1", file_count: 3, asset_count: 2, summary: null, updated_at: 1787000000 }]);
    mockedApi.fetchDatasetDetail.mockResolvedValue({
      id: 1,
      name: "Oryza_sativa_v1",
      type: "genome",
      species: "Oryza sativa",
      species_confidence: null,
      summary: null,
      path: "/a/v1",
      updated_at: 1787000000,
      assets: [{ id: 7, name: "assembly", type: "assembly", file_count: 3 }],
    });
    mockedApi.fetchFiles.mockResolvedValue({
      data: [
        { id: 101, asset_id: 7, name: "ref.fa", size: 123, role: null, mime_type: null, source_server: "srv", path: "/a/v1/ref.fa" },
      ],
      meta: { limit: 50, next_cursor: null, has_more: false },
    });
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "水稻" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    fireEvent.click(await screen.findByText("Oryza_sativa_v1"));
    expect(await screen.findByText("资产")).toBeInTheDocument();
    expect(screen.getByText("assembly（assembly）· 3 文件")).toBeInTheDocument();
    expect(screen.getByText("/a/v1/ref.fa")).toBeInTheDocument();
    expect(mockedApi.fetchDatasetDetail).toHaveBeenCalledWith(1);
    expect(mockedApi.fetchFiles).toHaveBeenCalledWith(1);
    // 弹层内共享按钮可用（复用 DatasetDetailModal）
    expect(screen.getByRole("button", { name: /共享/ })).toBeEnabled();
  });

  // GUI-T5: 搜索页同样具备共享能力——共享状态提升到页面级后，
  // 从搜索结果弹层发起的共享仍完整驱动（配对码/进度面板）。
  it("starts a share from the result modal and shows the pairing code", async () => {
    mockedApi.searchDatasets.mockResolvedValue([{ id: 1, name: "Oryza_sativa_v1", type: "genome", species: "Oryza sativa", path: "/a/v1", file_count: 3, asset_count: 2, summary: null, updated_at: 1787000000 }]);
    mockedApi.fetchDatasetDetail.mockResolvedValue({
      id: 1,
      name: "Oryza_sativa_v1",
      type: "genome",
      species: "Oryza sativa",
      species_confidence: null,
      summary: null,
      path: "/a/v1",
      updated_at: 1787000000,
      assets: [{ id: 7, name: "assembly", type: "assembly", file_count: 3 }],
    });
    mockedApi.fetchFiles.mockResolvedValue({
      data: [
        { id: 101, asset_id: 7, name: "ref.fa", size: 123, role: null, mime_type: null, source_server: "srv", path: "/a/v1/ref.fa" },
      ],
      meta: { limit: 50, next_cursor: null, has_more: false },
    });
    vi.mocked(invoke).mockResolvedValue(null);
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), { target: { value: "水稻" } });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    fireEvent.click(await screen.findByText("Oryza_sativa_v1"));
    fireEvent.click(await screen.findByRole("button", { name: /共享/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("share_dataset", { path: "/a/v1", ttlHours: 168 })
    );
    eventMock.emit("share://code", "8-purple-hammer");
    expect(await screen.findByText("8-purple-hammer")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /取消传输/ })).toBeInTheDocument();
  });

  // ---------- NR-T5: 对话搜索（有模型 → 对话模式；无模型 → 基础模式；LLM 失败 → 降级） ----------

  // read_config 返回形状（与后端 FanConfig 同构）；api_key 非空 = 已配置模型
  const llmCfg = {
    threads: null,
    include: [],
    exclude: [],
    endpoint: "https://api.example.com/v1",
    api_key: "sk-test",
    model: "gpt-4o-mini",
    api_type: "openai",
  };
  const result1 = {
    id: 1,
    name: "Oryza_sativa_v1",
    type: "genome",
    species: "Oryza sativa",
    path: "/a/v1",
    file_count: 3,
    asset_count: 2,
    summary: null,
    updated_at: 1787000000,
  };

  it("renders the conversation UI when llm is configured", async () => {
    vi.mocked(invoke).mockResolvedValue(llmCfg);
    render(<SearchPage />);
    expect(await screen.findByPlaceholderText(/用自然语言描述/)).toBeInTheDocument();
    // 对话模式无基础搜索框、无"未配置模型"提示
    expect(screen.queryByPlaceholderText(/搜索你的数据/)).not.toBeInTheDocument();
    expect(screen.queryByText(/未配置模型/)).not.toBeInTheDocument();
  });

  it("asks via chatSearch and renders results + expandable llm query", async () => {
    vi.mocked(invoke).mockResolvedValue(llmCfg);
    mockedApi.chatSearch.mockResolvedValue({
      query: { keywords: ["水稻", "基因组"], type: "genome" },
      results: [result1],
    });
    render(<SearchPage />);
    const input = await screen.findByPlaceholderText(/用自然语言描述/);
    fireEvent.change(input, { target: { value: "帮我找水稻基因组" } });
    fireEvent.click(screen.getByRole("button", { name: /发送/ }));
    // 首轮：历史为空
    await waitFor(() =>
      expect(mockedApi.chatSearch).toHaveBeenCalledWith([], "帮我找水稻基因组")
    );
    expect(await screen.findByText("Oryza_sativa_v1")).toBeInTheDocument();
    // LLM 查询可展开展示（关键词 + 类型）
    expect(screen.getByText(/水稻、基因组/)).toBeInTheDocument();
    // 追问：第二轮带上历史消息（user 问题 + assistant 摘要）
    fireEvent.change(input, { target: { value: "再找转录组" } });
    fireEvent.click(screen.getByRole("button", { name: /发送/ }));
    await waitFor(() =>
      expect(mockedApi.chatSearch).toHaveBeenLastCalledWith(
        [
          { role: "user", content: "帮我找水稻基因组" },
          { role: "assistant", content: "找到 1 个数据集" },
        ],
        "再找转录组"
      )
    );
  });

  it("renders basic search with hint when llm is not configured", async () => {
    vi.mocked(invoke).mockResolvedValue({ ...llmCfg, api_key: "" });
    render(<SearchPage />);
    expect(await screen.findByPlaceholderText(/搜索你的数据/)).toBeInTheDocument();
    expect(screen.getByText(/未配置模型，使用基础搜索/)).toBeInTheDocument();
    // 基础搜索仍可用
    mockedApi.searchDatasets.mockResolvedValue([result1]);
    fireEvent.change(screen.getByPlaceholderText(/搜索你的数据/), {
      target: { value: "水稻" },
    });
    fireEvent.click(screen.getByRole("button", { name: /搜索/ }));
    expect(await screen.findByText("Oryza_sativa_v1")).toBeInTheDocument();
  });

  it("falls back to basic search with a hint when the llm call fails", async () => {
    vi.mocked(invoke).mockResolvedValue(llmCfg);
    mockedApi.chatSearch.mockRejectedValue(new Error("HTTP 503"));
    mockedApi.searchDatasets.mockResolvedValue([result1]);
    render(<SearchPage />);
    const input = await screen.findByPlaceholderText(/用自然语言描述/);
    fireEvent.change(input, { target: { value: "水稻" } });
    fireEvent.click(screen.getByRole("button", { name: /发送/ }));
    expect(await screen.findByText(/模型调用失败，已切换基础搜索/)).toBeInTheDocument();
    expect(mockedApi.searchDatasets).toHaveBeenCalledWith("水稻");
    expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument();
  });
});
