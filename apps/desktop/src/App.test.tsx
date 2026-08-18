import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import App from "./App";
import * as api from "./api";

vi.mock("./api");
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// T17：App 挂载 HomePage → ScanPanel 订阅 scan:// 事件，jsdom 无 Tauri IPC，
// 必须 mock 掉。
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

import { invoke } from "@tauri-apps/api/core";

// Tauri 命令 mock 分发：App 挂载即调用 engine_error / get_share_port，
// 页面各自调用 read_config 等，按命令名返回对应载荷。
function mockInvoke(overrides: Record<string, unknown> = {}) {
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd in overrides) return Promise.resolve(overrides[cmd]);
    switch (cmd) {
      case "read_config":
        return Promise.resolve({
          include: ["/data/kentnf/orders"],
          exclude: [],
          endpoint: "",
          api_key: "",
          model: "",
        });
      case "engine_error":
        return Promise.resolve(null);
      case "get_share_port":
        return Promise.resolve(17951);
      case "retry_engine":
        return Promise.resolve(17951);
      case "scan_state":
        return Promise.resolve(false);
      default:
        return Promise.resolve(undefined);
    }
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  mockInvoke();
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
  it("sets api base from get_share_port", async () => {
    render(<App />);
    await screen.findByText("🔄 重新扫描");
    expect(invoke).toHaveBeenCalledWith("get_share_port");
    expect(api.setApiBase).toHaveBeenCalledWith(17951);
  });
});

describe("engine banner", () => {
  it("shows banner when engine_error returns a message and retry clears it", async () => {
    mockInvoke({ engine_error: "引擎未运行" });
    render(<App />);
    await screen.findByText(/引擎未运行/);
    // 轮询在挂载时同步了一次 engine_error
    expect(invoke).toHaveBeenCalledWith("engine_error");
    fireEvent.click(screen.getByText("重试"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("retry_engine"));
    await waitFor(() => expect(screen.queryByText(/引擎未运行/)).not.toBeInTheDocument());
  });
});
