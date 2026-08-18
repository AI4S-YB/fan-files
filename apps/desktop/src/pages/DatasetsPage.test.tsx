import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import DatasetsPage from "./DatasetsPage";
import * as api from "../api";

vi.mock("../api");
// T13: 打开目录走 Tauri open_path 命令
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";

const mockedApi = vi.mocked(api);

const summary1 = {
  id: 1,
  name: "Oryza_sativa_v1",
  type: "genome",
  species: "Oryza sativa",
  summary: null,
  path: "/data/orders/rice",
  asset_count: 2,
  file_count: 42,
  updated_at: 1724000000,
};
const summary2 = {
  id: 2,
  name: "Corn_panel",
  type: null,
  species: null,
  summary: null,
  path: null,
  asset_count: 0,
  file_count: 0,
  updated_at: 1724000001,
};
const pageOne = {
  data: [summary1, summary2],
  meta: {
    limit: 50,
    next_cursor: 2,
    has_more: true,
    sort: "id",
    type_counts: [
      { value: "genome", count: 12 },
      { value: "transcriptome", count: 7 },
      { value: "variant", count: 3 },
      { value: "other", count: 1 },
    ],
  },
};
const detailFixture = {
  id: 1,
  name: "Oryza_sativa_v1",
  type: "genome",
  species: "Oryza sativa",
  species_confidence: null,
  summary: null,
  path: "/data/orders/rice",
  updated_at: 1724000000,
  assets: [{ id: 7, name: "assembly", type: "assembly", file_count: 3 }],
};
const filesPage = {
  data: [
    {
      id: 101,
      asset_id: 7,
      name: "ref.fa",
      size: 123,
      role: null,
      mime_type: null,
      source_server: "srv",
      path: "/data/orders/rice/ref.fa",
    },
  ],
  meta: { limit: 50, next_cursor: null, has_more: false },
};

beforeEach(() => {
  // resetAllMocks：连未消费的 mockResolvedValueOnce 队列也清掉，避免一个用例中途失败污染后续用例
  vi.resetAllMocks();
  mockedApi.fetchDatasets.mockResolvedValue(pageOne);
  mockedApi.fetchDatasetDetail.mockResolvedValue(detailFixture);
  mockedApi.fetchFiles.mockResolvedValue(filesPage);
});

describe("DatasetsPage", () => {
  it("renders rows and type chips from meta.type_counts", async () => {
    render(<DatasetsPage />);
    expect(await screen.findByText("Oryza_sativa_v1")).toBeInTheDocument();
    expect(screen.getByText("Corn_panel")).toBeInTheDocument();
    // 表头
    for (const head of ["名称", "类型", "物种", "文件", "路径"]) {
      expect(screen.getByRole("columnheader", { name: head })).toBeInTheDocument();
    }
    // chips 用后端聚合的 type_counts（含数量）
    expect(screen.getByText("genome (12)")).toBeInTheDocument();
    expect(screen.getByText("transcriptome (7)")).toBeInTheDocument();
    expect(screen.getByText("variant (3)")).toBeInTheDocument();
    expect(screen.getByText("other (1)")).toBeInTheDocument();
    expect(mockedApi.fetchDatasets).toHaveBeenCalledWith({ cursor: undefined, limit: 50, type: undefined });
  });

  it("opens detail modal with assets and files on row click", async () => {
    render(<DatasetsPage />);
    fireEvent.click(await screen.findByText("Oryza_sativa_v1"));
    expect(await screen.findByText("资产")).toBeInTheDocument();
    expect(screen.getByText("assembly（assembly）· 3 文件")).toBeInTheDocument();
    expect(screen.getByText("/data/orders/rice/ref.fa")).toBeInTheDocument();
    expect(mockedApi.fetchDatasetDetail).toHaveBeenCalledWith(1);
    expect(mockedApi.fetchFiles).toHaveBeenCalledWith(1);
    const share = screen.getByRole("button", { name: /共享/ });
    // 详情带本地路径时"共享"可用（P2P 已接入）
    expect(share).toBeEnabled();
    expect(share).toHaveAttribute("title", "生成配对码，对方凭码接收");
    // T13 接入后：详情带本地路径时"打开目录"可用
    const openDir = screen.getByRole("button", { name: /打开目录/ });
    expect(openDir).toBeEnabled();
  });

  it("opens the dataset directory via open_path from the detail modal", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    render(<DatasetsPage />);
    fireEvent.click(await screen.findByText("Oryza_sativa_v1"));
    await screen.findByText("资产");
    const openDir = screen.getByRole("button", { name: /打开目录/ });
    fireEvent.click(openDir);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("open_path", { path: "/data/orders/rice" })
    );
  });

  it("keeps 打开目录 disabled when the dataset has no local path", async () => {
    mockedApi.fetchDatasetDetail.mockResolvedValue({ ...detailFixture, path: null });
    render(<DatasetsPage />);
    fireEvent.click(await screen.findByText("Oryza_sativa_v1"));
    await screen.findByText("资产");
    expect(screen.getByRole("button", { name: /打开目录/ })).toBeDisabled();
  });

  it("disables type chips while a page request is in flight", async () => {
    let resolvePage!: (p: typeof pageOne) => void;
    mockedApi.fetchDatasets.mockReturnValue(
      new Promise((r) => {
        resolvePage = r;
      }) as never
    );
    render(<DatasetsPage />);
    // 请求在途（loading=true）：chips 与翻页按钮都禁用，防止切筛选后旧响应覆盖 UI。
    // 注意在途时 typeCounts 尚未到达，chip 显示回退集文案（无计数）
    await waitFor(() => expect(screen.getByRole("button", { name: "genome" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "上一页" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "下一页" })).toBeDisabled();
    resolvePage(pageOne);
    expect(await screen.findByText("Oryza_sativa_v1")).toBeInTheDocument();
    expect(screen.getByText("genome (12)")).toBeEnabled();
  });

  it("disables next page button when next_cursor is null", async () => {
    mockedApi.fetchDatasets.mockResolvedValue({
      data: [summary1],
      meta: { limit: 50, next_cursor: null, has_more: false },
    });
    render(<DatasetsPage />);
    await screen.findByText("Oryza_sativa_v1");
    expect(screen.getByRole("button", { name: "下一页" })).toBeDisabled();
  });

  it("loads next page with cursor on button click", async () => {
    mockedApi.fetchDatasets
      .mockResolvedValueOnce(pageOne)
      .mockResolvedValueOnce({
        data: [{ ...summary1, id: 51, name: "Second_page_dataset" }],
        meta: { limit: 50, next_cursor: null, has_more: false },
      });
    render(<DatasetsPage />);
    await screen.findByText("Oryza_sativa_v1");
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    expect(await screen.findByText("Second_page_dataset")).toBeInTheDocument();
    expect(mockedApi.fetchDatasets).toHaveBeenLastCalledWith({ cursor: 2, limit: 50, type: undefined });
  });

  it("disables 上一页 until a page is pushed, then returns to page 1 with undefined cursor", async () => {
    mockedApi.fetchDatasets
      .mockResolvedValueOnce(pageOne)
      .mockResolvedValueOnce({
        data: [{ ...summary1, id: 51, name: "Second_page_dataset" }],
        meta: { limit: 50, next_cursor: 3, has_more: true },
      })
      .mockResolvedValueOnce(pageOne);
    render(<DatasetsPage />);
    await screen.findByText("Oryza_sativa_v1");
    const prev = screen.getByRole("button", { name: "上一页" });
    expect(prev).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    expect(await screen.findByText("Second_page_dataset")).toBeInTheDocument();
    expect(prev).toBeEnabled();
    fireEvent.click(prev);
    expect(await screen.findByText("Oryza_sativa_v1")).toBeInTheDocument();
    expect(mockedApi.fetchDatasets).toHaveBeenCalledTimes(3);
    // 返回第一页：cursor 为 undefined
    expect(mockedApi.fetchDatasets).toHaveBeenLastCalledWith({ cursor: undefined, limit: 50, type: undefined });
    // 历史栈清空后上一页再次禁用
    expect(prev).toBeDisabled();
  });

  it("back from third page loads with the first stacked cursor", async () => {
    const page2 = {
      data: [{ ...summary1, id: 51, name: "P2" }],
      meta: { limit: 50, next_cursor: 3, has_more: true },
    };
    const page3 = {
      data: [{ ...summary1, id: 101, name: "P3" }],
      meta: { limit: 50, next_cursor: null, has_more: false },
    };
    mockedApi.fetchDatasets
      .mockResolvedValueOnce(pageOne)
      .mockResolvedValueOnce(page2)
      .mockResolvedValueOnce(page3)
      .mockResolvedValueOnce(page2);
    render(<DatasetsPage />);
    await screen.findByText("Oryza_sativa_v1");
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    await screen.findByText("P2");
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    await screen.findByText("P3");
    fireEvent.click(screen.getByRole("button", { name: "上一页" }));
    await screen.findByText("P2");
    expect(mockedApi.fetchDatasets).toHaveBeenCalledTimes(4);
    // 栈 [2,3] → pop 3 → 用栈顶 cursor 2 重新 fetch（不是 undefined）
    expect(mockedApi.fetchDatasets).toHaveBeenLastCalledWith({ cursor: 2, limit: 50, type: undefined });
  });

  it("filters by type chip and toggles off", async () => {
    render(<DatasetsPage />);
    await screen.findByText("Oryza_sativa_v1");
    const chip = screen.getByText("variant (3)");
    fireEvent.click(chip);
    expect(await screen.findByText("variant (3)")).toHaveClass("chip", "active");
    expect(mockedApi.fetchDatasets).toHaveBeenLastCalledWith({ cursor: undefined, limit: 50, type: "variant" });
    fireEvent.click(screen.getByText("variant (3)"));
    await screen.findByText("Oryza_sativa_v1");
    expect(mockedApi.fetchDatasets).toHaveBeenLastCalledWith({ cursor: undefined, limit: 50, type: undefined });
    expect(screen.getByText("variant (3)")).not.toHaveClass("active");
  });

  it("shows empty state when fetch fails", async () => {
    mockedApi.fetchDatasets.mockRejectedValue(new Error("HTTP 503"));
    render(<DatasetsPage />);
    expect(await screen.findByText(/还没有数据集/)).toBeInTheDocument();
  });

  it("shares the dataset via P2P and shows the pairing code", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    render(<DatasetsPage />);
    fireEvent.click(await screen.findByText("Oryza_sativa_v1"));
    await screen.findByText("资产");
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("share_dataset", { path: "/data/orders/rice" })
    );
    // 传输中显示连接提示
    expect(screen.getByText(/正在连接/)).toBeInTheDocument();
  });
});
