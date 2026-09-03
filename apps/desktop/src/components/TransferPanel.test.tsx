import { render, screen, fireEvent, act } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import TransferPanel, { type TransferEvent } from "./TransferPanel";

// TDD 测试：事件 → 面板各展示位的驱动（进度条 / 连接徽标 / 续传标识 / 取消）
function renderPanel(events: TransferEvent[], log: string[] = []) {
  const onCancel = vi.fn();
  const utils = render(
    <TransferPanel name="rice.tar.gz" events={events} log={log} onCancel={onCancel} />
  );
  return { onCancel, rerender: utils.rerender };
}

describe("TransferPanel", () => {
  it("shows file name and formatted total size", () => {
    renderPanel([{ type: "progress", sent: 512, total: 2 * 1024 * 1024, pct: 0.02, chunks: 1 }]);
    expect(screen.getByText(/rice\.tar\.gz/)).toBeInTheDocument();
    expect(screen.getByText(/2\.0 MB/)).toBeInTheDocument();
  });

  it("renders progress bar from progress event (pct + sent/total)", () => {
    renderPanel([{ type: "progress", sent: 512, total: 1024, pct: 50, chunks: 1 }]);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "50");
    expect(bar).toHaveStyle({ width: "50%" });
    expect(screen.getByText(/512 B \/ 1\.0 KB/)).toBeInTheDocument();
  });

  it("renders conn badge: direct → P2P直连", () => {
    renderPanel([{ type: "conn", mode: "direct" }]);
    expect(screen.getByText("P2P直连")).toBeInTheDocument();
  });
  it("renders conn badge: relay → 中继relay", () => {
    renderPanel([{ type: "conn", mode: "relay" }]);
    expect(screen.getByText("中继relay")).toBeInTheDocument();
  });
  it("renders conn badge: punching → 打洞中", () => {
    renderPanel([{ type: "conn", mode: "punching" }]);
    expect(screen.getByText("打洞中")).toBeInTheDocument();
  });

  it("renders resume badge with percentage", () => {
    renderPanel([{ type: "resume", done: 34, total: 120 }]);
    expect(screen.getByText(/已恢复 28%/)).toBeInTheDocument();
  });

  it("shows done state and disables cancel on success", () => {
    const { onCancel } = renderPanel([
      { type: "done", ok: true, bytes: 1024, elapsed_secs: 3.2 },
    ]);
    expect(screen.getByText(/传输完成/)).toBeInTheDocument();
    expect(screen.getByText(/1\.0 KB/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /取消/ })).toBeDisabled();
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("shows error message from error event", () => {
    renderPanel([{ type: "error", msg: "传输失败: 连接超时" }]);
    expect(screen.getByText(/传输失败: 连接超时/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /取消/ })).toBeDisabled();
  });

  it("calls onCancel when cancel button is clicked", () => {
    const { onCancel } = renderPanel([{ type: "conn", mode: "punching" }]);
    fireEvent.click(screen.getByRole("button", { name: /取消/ }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("shows raw log lines inside the collapsed details", () => {
    renderPanel(
      [{ type: "conn", mode: "direct" }],
      ['{"type":"conn","mode":"direct"}', "配对码: 8-purple-hammer"]
    );
    expect(screen.getByText(/原始日志/)).toBeInTheDocument();
    expect(screen.getByText("配对码: 8-purple-hammer")).toBeInTheDocument();
  });

  // GUI-T3 修复: 新传输（事件流清空）时重置速度样本，旧传输样本不污染新 ETA。
  // 传输1: 0→100B/1s（剩余9s）；新传输清空后: 0→1000B/1s（剩余1s）。
  // 若未重置，混入旧样本后 ETA 会算出"剩余 2 秒"——用文案差异证明重置生效。
  it("resets speed samples when a new transfer starts (events cleared)", () => {
    vi.useFakeTimers();
    try {
      const p = (sent: number, total: number): TransferEvent => ({
        type: "progress",
        sent,
        total,
        pct: (sent / total) * 100,
        chunks: 1,
      });
      const { rerender } = renderPanel([p(0, 1000)]);
      act(() => vi.advanceTimersByTime(1000));
      rerender(
        <TransferPanel
          name="a.tar"
          events={[p(0, 1000), p(100, 1000)]}
          log={[]}
          onCancel={vi.fn()}
        />
      );
      act(() => vi.advanceTimersByTime(1000));
      // 第 3 个事件渲染时才有 2 个速度样本 → ETA 出现
      rerender(
        <TransferPanel
          name="a.tar"
          events={[p(0, 1000), p(100, 1000), p(200, 1000)]}
          log={[]}
          onCancel={vi.fn()}
        />
      );
      expect(screen.getByText("剩余 8 秒")).toBeInTheDocument();
      act(() => vi.advanceTimersByTime(1000));
      // 新传输：事件流清空 → 速度样本重置
      rerender(
        <TransferPanel name="b.tar" events={[]} log={[]} onCancel={vi.fn()} />
      );
      rerender(
        <TransferPanel
          name="b.tar"
          events={[p(0, 2000)]}
          log={[]}
          onCancel={vi.fn()}
        />
      );
      act(() => vi.advanceTimersByTime(1000));
      rerender(
        <TransferPanel
          name="b.tar"
          events={[p(0, 2000), p(1000, 2000)]}
          log={[]}
          onCancel={vi.fn()}
        />
      );
      act(() => vi.advanceTimersByTime(1000));
      rerender(
        <TransferPanel
          name="b.tar"
          events={[p(0, 2000), p(1000, 2000), p(1500, 2000)]}
          log={[]}
          onCancel={vi.fn()}
        />
      );
      // 重置后样本为 0@3s/1000@4s → 1s；混入旧样本则 (2000-1500)/(1000/4s)=2s
      expect(screen.getByText("剩余 1 秒")).toBeInTheDocument();
      expect(screen.queryByText("剩余 2 秒")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
