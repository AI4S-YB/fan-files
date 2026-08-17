import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SearchPage from "./SearchPage";
import * as api from "../api";

vi.mock("../api");
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
});
