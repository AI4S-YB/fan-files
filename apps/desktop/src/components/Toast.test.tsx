import { render, screen, fireEvent, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import ToastProvider, { pushToast } from "./Toast";

function renderHarness() {
  return render(<ToastProvider><div /></ToastProvider>);
}

afterEach(() => {
  vi.useRealTimers();
});

describe("Toast", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  it("renders the message after pushToast and auto-dismisses after 4s", () => {
    renderHarness();
    act(() => pushToast({ kind: "info", message: "扫描完成" }));
    expect(screen.getByText("扫描完成")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(4000));
    expect(screen.queryByText("扫描完成")).not.toBeInTheDocument();
  });

  it("stacks multiple toasts and dismisses each after 4s", () => {
    renderHarness();
    act(() => pushToast({ kind: "info", message: "第一条" }));
    act(() => pushToast({ kind: "info", message: "第二条" }));
    expect(screen.getByText("第一条")).toBeInTheDocument();
    expect(screen.getByText("第二条")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(4000));
    expect(screen.queryByText("第一条")).not.toBeInTheDocument();
    expect(screen.queryByText("第二条")).not.toBeInTheDocument();
  });

  it("dismisses immediately when the close button is clicked", () => {
    renderHarness();
    act(() => pushToast({ kind: "error", message: "扫描失败，详见日志" }));
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    expect(screen.queryByText("扫描失败，详见日志")).not.toBeInTheDocument();
  });

  it("applies the data-kind attribute for success/error/info", () => {
    renderHarness();
    act(() => pushToast({ kind: "success", message: "已完成" }));
    const ok = screen.getByText("已完成").closest("[data-kind]");
    expect(ok).toHaveAttribute("data-kind", "success");
    act(() => pushToast({ kind: "error", message: "出错了" }));
    const err = screen.getByText("出错了").closest("[data-kind]");
    expect(err).toHaveAttribute("data-kind", "error");
    act(() => pushToast({ kind: "info", message: "默认提示" }));
    const info = screen.getByText("默认提示").closest("[data-kind]");
    expect(info).toHaveAttribute("data-kind", "info");
  });
});
