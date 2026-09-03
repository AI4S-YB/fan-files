import { render, screen, fireEvent, waitFor, within, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import App from "./App";
import * as api from "./api";

// 只 mock 网络函数；setApiBase/getApiBase 保留真实实现，
// 以便断言 retry 后 base 确实更新到 retry_engine 返回的端口。
vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return { ...actual, fetchStats: vi.fn() };
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// T17：App 挂载 HomePage → ScanPanel 订阅 scan:// 事件，jsdom 无 Tauri IPC，
// 必须 mock 掉。SF-T3：捕获监听器以便测试注入 scan://done 断言 fan-scan-done 广播。
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
  eventMock.clear();
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
    expect(api.getApiBase()).toBe("http://127.0.0.1:17951");
  });
  it("home empty-state CTA jumps to the settings page", async () => {
    // 无目录配置 → HomePage 显示空态 CTA；点击后应切到设置页（出现"保存配置"按钮）
    mockInvoke({
      read_config: { include: [], exclude: [], endpoint: "", api_key: "", model: "" },
    });
    render(<App />);
    fireEvent.click(await screen.findByText(/选择目录开始扫描/));
    expect(
      await screen.findByRole("button", { name: "保存配置" })
    ).toBeInTheDocument();
  });

  // SF-T3: App 级 scan://done 监听 → 广播 fan-scan-done 全局事件（页面联动刷新）。
  // 只广播成功（payload=0）；失败退出码不广播。
  it("broadcasts fan-scan-done when scan://done succeeds (payload 0)", async () => {
    let heard = 0;
    const onScan = () => heard++;
    window.addEventListener("fan-scan-done", onScan);
    try {
      render(<App />);
      await screen.findByText("🔄 重新扫描");
      act(() => eventMock.emit("scan://done", 0));
      expect(heard).toBe(1);
      act(() => eventMock.emit("scan://done", 1));
      expect(heard).toBe(1);
    } finally {
      window.removeEventListener("fan-scan-done", onScan);
    }
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

  it("retry updates api base to the port returned by retry_engine", async () => {
    // 冲突回退场景：retry_engine 返回不同于初始端口的新端口
    mockInvoke({ engine_error: "引擎未运行", retry_engine: 30284 });
    render(<App />);
    await screen.findByText(/引擎未运行/);
    await waitFor(() => expect(api.getApiBase()).toBe("http://127.0.0.1:17951"));
    fireEvent.click(screen.getByText("重试"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("retry_engine"));
    await waitFor(() => expect(api.getApiBase()).toBe("http://127.0.0.1:30284"));
  });
});
