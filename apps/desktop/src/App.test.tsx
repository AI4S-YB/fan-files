import { render, screen, fireEvent, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import App from "./App";
import * as api from "./api";

vi.mock("./api");
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(invoke).mockResolvedValue({
    include: ["/data/kentnf/orders"],
    exclude: [],
    endpoint: "",
    api_key: "",
    model: "",
  });
  vi.mocked(api.fetchStats).mockResolvedValue(null as never);
});

describe("App shell", () => {
  it("renders sidebar with four entries", async () => {
    const { container } = render(<App />);
    // HomePage 挂载后有异步状态更新，等 ScanPanel 出现以 flush 掉它们
    await screen.findByText("🔄 重新扫描");
    // 页面正文里也有"首页"等文案（如 h2），断言范围限定在侧边栏内
    const sidebar = within(container.querySelector(".sidebar") as HTMLElement);
    for (const label of ["首页", "数据集", "搜索", "设置"]) {
      expect(sidebar.getByText(label)).toBeInTheDocument();
    }
  });
  it("switches page on sidebar click", async () => {
    const { container } = render(<App />);
    await screen.findByText("🔄 重新扫描");
    const sidebar = within(container.querySelector(".sidebar") as HTMLElement);
    const btn = sidebar.getByText("数据集").closest("button")!;
    fireEvent.click(btn);
    expect(btn).toHaveClass("side-item", "active");
  });
});
