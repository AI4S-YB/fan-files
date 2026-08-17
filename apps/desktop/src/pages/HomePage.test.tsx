import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import HomePage from "./HomePage";
import * as api from "../api";

vi.mock("../api");
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

const mockedApi = vi.mocked(api);

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockResolvedValue({
    include: ["/data/kentnf/orders"],
    exclude: [],
    endpoint: "",
    api_key: "",
    model: "",
  });
});

describe("HomePage", () => {
  it("shows empty-state CTA when no directories configured", async () => {
    vi.mocked(invoke).mockResolvedValue({
      include: [],
      exclude: [],
      endpoint: "",
      api_key: "",
      model: "",
    });
    mockedApi.fetchStats.mockResolvedValue(null as never); // 无索引
    render(<HomePage />);
    // 按钮文案为 "📁 选择目录开始扫描"，用正则做子串匹配
    expect(await screen.findByText(/选择目录开始扫描/)).toBeInTheDocument();
  });

  it("renders stat cards when indexed", async () => {
    mockedApi.fetchStats.mockResolvedValue({
      datasets_upper_bound: 1453,
      files_upper_bound: 109796,
      assets_upper_bound: 5000,
      linked_files_upper_bound: 100000,
      last_indexed_at: 1787000000,
      approximate: false,
    });
    render(<HomePage />);
    expect(await screen.findByText("1,453")).toBeInTheDocument();
    expect(screen.getByText("109,796")).toBeInTheDocument();
  });
});
