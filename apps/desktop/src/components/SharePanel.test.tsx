import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SharePanel from "./SharePanel";

// GUI-T5: 共享面板（配对码 + 传输面板）从 DatasetDetailModal 提取为独立组件，
// 页面级与弹层内共用——配对码复制反馈、进度渲染、取消回调在此覆盖。
describe("SharePanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows the pairing code with copy feedback", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    render(
      <SharePanel
        name="Oryza_sativa_v1"
        code="8-purple-hammer"
        events={[]}
        log={[]}
        onCancel={vi.fn()}
      />
    );
    expect(screen.getByText("8-purple-hammer")).toBeInTheDocument();
    expect(screen.getByText(/7 天内有效/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /复制/ }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("8-purple-hammer"));
    expect(await screen.findByText("已复制 ✓")).toBeInTheDocument();
  });

  it("renders progress events in the transfer panel", () => {
    render(
      <SharePanel
        name="Oryza_sativa_v1"
        code="8-purple-hammer"
        events={[
          { type: "conn", mode: "direct" },
          { type: "progress", sent: 512, total: 1024, pct: 50, chunks: 1 },
        ] as never}
        log={[]}
        onCancel={vi.fn()}
      />
    );
    expect(screen.getByText("P2P直连")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
  });

  it("invokes onCancel from the transfer panel cancel button", () => {
    const onCancel = vi.fn();
    render(
      <SharePanel
        name="Oryza_sativa_v1"
        events={[]}
        log={[]}
        onCancel={onCancel}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: /取消传输/ }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
