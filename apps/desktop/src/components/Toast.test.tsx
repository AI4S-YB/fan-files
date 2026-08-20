import { render, screen, fireEvent, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import ToastProvider, { useToast, type ToastType } from "./Toast";

// 测试探针：在 Provider 内挂一个组件拿到 showToast 引用，向外部暴露。
let showToast: (msg: string, type?: ToastType) => void = () => {};
function Probe() {
  showToast = useToast().showToast;
  return null;
}

function renderHarness() {
  return render(
    <ToastProvider>
      <Probe />
    </ToastProvider>
  );
}

beforeEach(() => {
  showToast = () => {};
});

afterEach(() => {
  vi.useRealTimers();
});

describe("Toast", () => {
  it("renders the message after showToast and auto-dismisses after 3s", () => {
    vi.useFakeTimers();
    renderHarness();
    act(() => showToast("扫描完成"));
    expect(screen.getByText("扫描完成")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3000));
    expect(screen.queryByText("扫描完成")).not.toBeInTheDocument();
  });

  it("stacks multiple toasts and dismisses each after 3s", () => {
    vi.useFakeTimers();
    renderHarness();
    act(() => showToast("第一条"));
    act(() => showToast("第二条"));
    expect(screen.getByText("第一条")).toBeInTheDocument();
    expect(screen.getByText("第二条")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(3000));
    expect(screen.queryByText("第一条")).not.toBeInTheDocument();
    expect(screen.queryByText("第二条")).not.toBeInTheDocument();
  });

  it("dismisses immediately when the close button is clicked", () => {
    vi.useFakeTimers();
    renderHarness();
    act(() => showToast("扫描失败，详见日志"));
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    expect(screen.queryByText("扫描失败，详见日志")).not.toBeInTheDocument();
  });

  it("applies the type class for coloring (success/error/info)", () => {
    vi.useFakeTimers();
    renderHarness();
    act(() => showToast("已完成", "success"));
    const ok = screen.getByText("已完成").closest(".toast-item");
    expect(ok!.className).toContain("toast-success");
    act(() => showToast("出错了", "error"));
    const err = screen.getByText("出错了").closest(".toast-item");
    expect(err!.className).toContain("toast-error");
    act(() => showToast("默认提示"));
    const info = screen.getByText("默认提示").closest(".toast-item");
    expect(info!.className).toContain("toast-info");
  });
});
