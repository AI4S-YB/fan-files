import { render, screen, act, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import HomePage from "./HomePage";
import * as api from "../api";

vi.mock("../api");
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// T17：ScanPanel 挂载即订阅 scan:// 事件（listen），jsdom 里没有 Tauri IPC，
// 必须 mock 掉。捕获回调以便模拟"扫描完成"驱动 refreshStats。
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { EventCallback } from "@tauri-apps/api/event";

const listeners = new Map<string, EventCallback<unknown>>();

function emit(name: string, payload: unknown) {
  const cb = listeners.get(name);
  if (!cb) throw new Error(`no listener registered for ${name}`);
  act(() => cb({ event: name, id: 0, payload }));
}

const mockedApi = vi.mocked(api);
const onGoSettings = vi.fn();

let mockConfig: Record<string, unknown> = {
  include: ["/data/kentnf/orders"],
  exclude: [],
  endpoint: "",
  api_key: "",
  model: "",
};

beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
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
  vi.mocked(listen).mockImplementation(
    (name: string, cb: EventCallback<unknown>) => {
      listeners.set(name, cb);
      return Promise.resolve(() => {});
    }
  );
});

describe("HomePage", () => {
  it("shows empty-state CTA when no directories configured", async () => {
    mockConfig = { ...mockConfig, include: [] };
    mockedApi.fetchStats.mockResolvedValue(null as never); // 无索引
    render(<HomePage onGoSettings={onGoSettings} />);
    // 按钮文案为 "📁 选择目录开始扫描"，用正则做子串匹配
    const cta = await screen.findByText(/选择目录开始扫描/);
    expect(cta).toBeInTheDocument();
    fireEvent.click(cta);
    expect(onGoSettings).toHaveBeenCalledTimes(1);
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
    render(<HomePage onGoSettings={onGoSettings} />);
    expect(await screen.findByText("1,453")).toBeInTheDocument();
    expect(screen.getByText("109,796")).toBeInTheDocument();
  });

  it("does not re-subscribe ScanPanel when stats refresh re-renders (stable onDone)", async () => {
    // 每次调用返回新对象：mockResolvedValue 复用同一引用会让 setStats 因
    // Object.is 相同而 bail out、重渲染根本不会发生，测不到重订阅行为。
    mockedApi.fetchStats.mockImplementation(async () => ({
      datasets_upper_bound: 1453,
      files_upper_bound: 109796,
      assets_upper_bound: 5000,
      linked_files_upper_bound: 100000,
      last_indexed_at: 1787000000,
      approximate: false,
    }));
    render(<HomePage onGoSettings={onGoSettings} />);
    await screen.findByText("1,453");
    // 模拟扫描完成：done(0) → onDone=refreshStats → setStats 触发重渲染。
    // onDone 身份稳定（useCallback）时 ScanPanel 不重订阅，listen 仍为 3 次；
    // 若 refreshStats 每次渲染都换新函数，重渲染会触发 +3 次重订阅。
    emit("scan://done", 0);
    await waitFor(() => expect(mockedApi.fetchStats).toHaveBeenCalledTimes(2));
    // 再 flush 一轮 act，让重渲染触发的（潜在）被动 effect 重订阅落地后再断言
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(vi.mocked(listen)).toHaveBeenCalledTimes(3);
  });
});
