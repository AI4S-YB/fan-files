import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import HomePage from "./HomePage";
import * as api from "../api";

vi.mock("../api");
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// T17：ScanPanel 挂载即订阅 scan:// 事件（listen），jsdom 里没有 Tauri IPC，
// 必须 mock 掉。
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

import { invoke } from "@tauri-apps/api/core";

const mockedApi = vi.mocked(api);

let mockConfig: Record<string, unknown> = {
  include: ["/data/kentnf/orders"],
  exclude: [],
  endpoint: "",
  api_key: "",
  model: "",
};

beforeEach(() => {
  vi.clearAllMocks();
  mockConfig = {
    include: ["/data/kentnf/orders"],
    exclude: [],
    endpoint: "",
    api_key: "",
    model: "",
  };
  // 按命令名分发：read_config 返回配置对象，scan_state 返回布尔（ScanPanel 轮询）
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd === "scan_state") return Promise.resolve(false);
    return Promise.resolve(mockConfig);
  });
});

describe("HomePage", () => {
  it("shows empty-state CTA when no directories configured", async () => {
    mockConfig = { ...mockConfig, include: [] };
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
