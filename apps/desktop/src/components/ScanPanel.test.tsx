import { render, screen, fireEvent, act, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import ScanPanel from "./ScanPanel";
import ToastProvider from "./Toast";

// 事件流模式（T17）：ScanPanel 挂载即 listen 三个 scan:// 事件并轮询一次
// scan_state。mock listen 捕获回调，测试用 emit() 模拟后端推送。
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// SF-T2：done(0) 后拉统计拼"发现 N 个数据集"文案；默认 mock 为失败，
// 让既有测试走"扫描完成"兜底路径、不碰真实网络。
vi.mock("../api", () => ({ fetchStats: vi.fn() }));

import { listen } from "@tauri-apps/api/event";
import type { EventCallback } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { fetchStats } from "../api";

// 真实 listen 的回调收到完整 Event<T>（event/id/payload），组件只用 payload。
const listeners = new Map<string, EventCallback<unknown>>();

function emit(name: string, payload: unknown) {
  const cb = listeners.get(name);
  if (!cb) throw new Error(`no listener registered for ${name}`);
  act(() => cb({ event: name, id: 0, payload }));
}

// flush 一轮微任务 + 宏任务：让 scan() 内的 await read_config → setRunning → scan_now 落地
async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

// LLM 预检默认放行（read_config 返回 api_key），既有点击扫描的测试无需改动
const CONFIG_WITH_KEY = {
  include: ["/data/kentnf/orders"],
  exclude: [],
  endpoint: "",
  api_key: "sk-test",
  model: "",
};

beforeEach(() => {
  vi.clearAllMocks();
  listeners.clear();
  vi.mocked(listen).mockImplementation(
    (name: string, cb: EventCallback<unknown>) => {
      listeners.set(name, cb);
      return Promise.resolve(() => {});
    }
  );
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd === "scan_state") return Promise.resolve(false);
    if (cmd === "read_config") return Promise.resolve(CONFIG_WITH_KEY);
    return Promise.resolve(undefined);
  });
  vi.mocked(fetchStats).mockRejectedValue(new Error("engine down"));
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("ScanPanel", () => {
  it("polls scan_state on mount and subscribes to scan:// events", async () => {
    render(<ScanPanel />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("scan_state"));
    expect(listeners.has("scan://progress")).toBe(true);
    expect(listeners.has("scan://done")).toBe(true);
    expect(listeners.has("scan://error")).toBe(true);
  });

  it("shows running state when scan_state is true", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "scan_state")
    );
    render(<ScanPanel />);
    await waitFor(() => expect(screen.getByRole("button")).toBeDisabled());
    expect(screen.getByRole("button")).toHaveTextContent("扫描中…");
  });

  it("clicking scan invokes scan_now and disables the button", async () => {
    render(<ScanPanel />);
    const btn = screen.getByRole("button");
    fireEvent.click(btn);
    // SF-T2：扫描前有 LLM 预检（await read_config），等待异步链路落地
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("scan_now"));
    expect(btn).toBeDisabled();
    expect(btn).toHaveTextContent("扫描中…");
  });

  it("done event with code 0 calls onDone and re-enables the button", async () => {
    const onDone = vi.fn();
    render(<ScanPanel onDone={onDone} />);
    fireEvent.click(screen.getByRole("button"));
    await flush();
    emit("scan://done", 0);
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button")).toBeEnabled();
  });

  it("done event with nonzero code does not call onDone", async () => {
    const onDone = vi.fn();
    render(<ScanPanel onDone={onDone} />);
    fireEvent.click(screen.getByRole("button"));
    await flush();
    emit("scan://done", 1);
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByRole("button")).toBeEnabled();
  });

  it("progress events append lines to the log", async () => {
    render(<ScanPanel />);
    emit("scan://progress", "  Analyzing directory structure: /tmp/fan-scan-test");
    emit("scan://progress", "  Phase B complete: 3 files indexed");
    const log = await screen.findByText(/Analyzing directory structure/);
    expect(log.textContent).toContain("Phase B complete: 3 files indexed");
  });

  it("error event appends failure line", async () => {
    render(<ScanPanel />);
    emit("scan://error", "spawn failed");
    expect(
      await screen.findByText("扫描失败: spawn failed")
    ).toBeInTheDocument();
  });

  it("error event re-enables the button (spawn failure must not leave it stuck)", async () => {
    render(<ScanPanel />);
    // 先让挂载时的 scan_state 轮询落地，避免其延迟的 setRunning(false)
    // 在点击后冲掉 running=true、掩盖缺少的 error 处理
    await flush();
    fireEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(screen.getByRole("button")).toBeDisabled());
    emit("scan://error", "spawn failed");
    expect(
      await screen.findByText("扫描失败: spawn failed")
    ).toBeInTheDocument();
    expect(screen.getByRole("button")).toBeEnabled();
  });

  it("caps the progress log at exactly 500 lines", () => {
    render(<ScanPanel />);
    for (let i = 1; i <= 505; i++) emit("scan://progress", `line ${i}`);
    const log = document.querySelector("pre.scan-log");
    expect(log).not.toBeNull();
    const logLines = log!.textContent!.split("\n");
    expect(logLines.length).toBe(500);
    expect(logLines[0]).toBe("line 6");
    expect(logLines[499]).toBe("line 505");
  });

  it("invoke rejection (e.g. already scanning) shows error and re-enables button", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_state") return Promise.resolve(false);
      if (cmd === "read_config") return Promise.resolve(CONFIG_WITH_KEY);
      return Promise.reject("already scanning");
    });
    render(<ScanPanel />);
    fireEvent.click(screen.getByRole("button"));
    expect(
      await screen.findByText("扫描失败: already scanning")
    ).toBeInTheDocument();
    expect(screen.getByRole("button")).toBeEnabled();
  });

  // ---- SF-T2：扫描完成/失败 toast + LLM 预检 ----

  it("done success: toast shows dataset count from fetchStats", async () => {
    vi.mocked(fetchStats).mockResolvedValue({
      datasets_upper_bound: 12,
      assets_upper_bound: 0,
      files_upper_bound: 0,
      linked_files_upper_bound: 0,
      last_indexed_at: null,
      approximate: false,
    });
    const onDone = vi.fn();
    render(
      <ToastProvider>
        <ScanPanel onDone={onDone} />
      </ToastProvider>
    );
    fireEvent.click(screen.getByRole("button"));
    await flush();
    emit("scan://done", 0);
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(
      await screen.findByText("扫描完成：发现 12 个数据集")
    ).toBeInTheDocument();
  });

  it("done success: fetchStats failure falls back to plain message", async () => {
    const onDone = vi.fn();
    render(
      <ToastProvider>
        <ScanPanel onDone={onDone} />
      </ToastProvider>
    );
    fireEvent.click(screen.getByRole("button"));
    await flush();
    emit("scan://done", 0);
    expect(onDone).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("扫描完成")).toBeInTheDocument();
  });

  it("done failure (nonzero code) shows error toast and skips onDone", async () => {
    const onDone = vi.fn();
    render(
      <ToastProvider>
        <ScanPanel onDone={onDone} />
      </ToastProvider>
    );
    fireEvent.click(screen.getByRole("button"));
    await flush();
    emit("scan://done", 1);
    expect(onDone).not.toHaveBeenCalled();
    expect(await screen.findByText("扫描失败，详见日志")).toBeInTheDocument();
  });

  it("scan://error shows error toast alongside the log line", async () => {
    render(
      <ToastProvider>
        <ScanPanel />
      </ToastProvider>
    );
    emit("scan://error", "spawn failed");
    expect(await screen.findByText("扫描失败，详见日志")).toBeInTheDocument();
    expect(
      await screen.findByText("扫描失败: spawn failed")
    ).toBeInTheDocument();
  });

  it("precheck: missing LLM api_key asks confirm; cancel aborts scan", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_state") return Promise.resolve(false);
      if (cmd === "read_config") {
        return Promise.resolve({ ...CONFIG_WITH_KEY, api_key: "" });
      }
      return Promise.resolve(undefined);
    });
    render(<ScanPanel />);
    fireEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(confirmSpy).toHaveBeenCalledTimes(1));
    expect(confirmSpy).toHaveBeenCalledWith(
      "未配置 LLM 模型，扫描将跳过数据集智能分类（Phase C），只做基础索引。是否继续？"
    );
    expect(invoke).not.toHaveBeenCalledWith("scan_now");
    expect(screen.getByRole("button")).toBeEnabled();
  });

  it("precheck: missing LLM api_key but user confirms — scan proceeds", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "scan_state") return Promise.resolve(false);
      if (cmd === "read_config") {
        return Promise.resolve({ ...CONFIG_WITH_KEY, api_key: "" });
      }
      return Promise.resolve(undefined);
    });
    render(<ScanPanel />);
    fireEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(confirmSpy).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("scan_now"));
  });

  it("precheck: LLM api_key configured scans directly without confirm", async () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<ScanPanel />);
    fireEvent.click(screen.getByRole("button"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("scan_now"));
    expect(confirmSpy).not.toHaveBeenCalled();
  });
});
