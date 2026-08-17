import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import DatasetsPage from "./DatasetsPage";
import * as api from "../api";

vi.mock("../api");

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
  vi.clearAllMocks();
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
    expect(share).toBeDisabled();
    expect(share).toHaveAttribute("title", "即将推出");
    expect(screen.getByRole("button", { name: /打开目录/ })).toBeInTheDocument();
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
});
