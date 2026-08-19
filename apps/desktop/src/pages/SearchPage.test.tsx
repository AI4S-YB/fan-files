import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SearchPage from "./SearchPage";
import * as api from "../api";

vi.mock("../api");
// GUI-T4: 详情弹层复用 DatasetDetailModal —— 其内部监听 share:// 事件并调用 Tauri
// invoke，jsdom 里没有 Tauri IPC，必须 mock 掉（listen 返回可清理的闭包即可）。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.resetAllMocks();
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
});
