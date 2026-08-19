import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import DatasetDetailModal from "./DatasetDetailModal";
import type { DatasetDetail, FileSummary } from "../api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";

// Tauri 事件流 mock：GUI-T4 提取后弹层自持 share:// 监听，
// 测试通过 eventMock.emit 注入 share:// 事件驱动共享面板与续传弹窗。
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

const detailFixture: DatasetDetail = {
  id: 1,
  name: "Oryza_sativa_v1",
  type: "genome",
  species: "Oryza sativa",
  species_confidence: null,
  summary: null,
  path: "/data/orders/rice",
  updated_at: 1724000000,
  assets: [{ id: 7, name: "assembly", type: "assembly", file_count: 3 }],
};
const filesFixture: FileSummary[] = [
  {
    id: 101,
    asset_id: 7,
    name: "ref.fa",
    size: 123,
    role: null,
    mime_type: null,
    source_server: "srv",
    path: "/data/orders/rice/ref.fa",
  },
];

function renderModal(overrides: { detail?: DatasetDetail; files?: FileSummary[] } = {}) {
  render(
    <DatasetDetailModal
      detail={overrides.detail ?? detailFixture}
      files={overrides.files ?? filesFixture}
      onClose={vi.fn()}
    />
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  eventMock.clear();
});

describe("DatasetDetailModal", () => {
  it("renders dataset detail with assets and files", () => {
    renderModal();
    expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument();
    expect(screen.getByText(/物种: Oryza sativa · 路径: \/data\/orders\/rice/)).toBeInTheDocument();
    expect(screen.getByText("资产")).toBeInTheDocument();
    expect(screen.getByText("assembly（assembly）· 3 文件")).toBeInTheDocument();
    expect(screen.getByText("/data/orders/rice/ref.fa")).toBeInTheDocument();
  });

  it("calls onClose when the backdrop is clicked", () => {
    const onClose = vi.fn();
    render(<DatasetDetailModal detail={detailFixture} files={filesFixture} onClose={onClose} />);
    fireEvent.click(screen.getByText("Oryza_sativa_v1").closest(".modal") as HTMLElement);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("enables 共享 when the dataset has a local path", () => {
    renderModal();
    const share = screen.getByRole("button", { name: /共享/ });
    expect(share).toBeEnabled();
    expect(share).toHaveAttribute("title", "生成配对码，对方凭码接收");
    // T13 接入后：详情带本地路径时"打开目录"可用
    expect(screen.getByRole("button", { name: /打开目录/ })).toBeEnabled();
  });

  it("keeps 打开目录 and 共享 disabled when the dataset has no local path", () => {
    renderModal({ detail: { ...detailFixture, path: null } });
    expect(screen.getByRole("button", { name: /打开目录/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /共享/ })).toBeDisabled();
  });

  it("opens the dataset directory via open_path", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /打开目录/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("open_path", { path: "/data/orders/rice" })
    );
  });

  it("shares the dataset via P2P and shows the pairing code", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("share_dataset", { path: "/data/orders/rice" })
    );
    expect(screen.getByText(/正在连接/)).toBeInTheDocument();
    eventMock.emit("share://code", "8-purple-hammer");
    expect(await screen.findByText("8-purple-hammer")).toBeInTheDocument();
  });

  // GUI-T3: progress/conn 事件驱动共享传输面板（进度条 + 连接徽标）
  it("drives the share panel from progress events", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    expect(await screen.findByText(/正在连接/)).toBeInTheDocument();
    eventMock.emit("share://progress", JSON.stringify({ type: "conn", mode: "punching" }));
    expect(await screen.findByText("打洞中")).toBeInTheDocument();
    eventMock.emit(
      "share://progress",
      JSON.stringify({ type: "progress", sent: 512, total: 1024, pct: 50, chunks: 1 })
    );
    await waitFor(() =>
      expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50")
    );
  });

  // GUI-T3: resume 事件 → 续传确认弹窗；拒绝 → cancel_transfer
  it("shows the resume confirm dialog and rejecting invokes cancel_transfer", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    eventMock.emit("share://progress", JSON.stringify({ type: "resume", done: 34, total: 120 }));
    expect(await screen.findByText(/发现未完成传输/)).toBeInTheDocument();
    expect(screen.getByText(/已收 34\/120/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "放弃并取消" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("cancel_transfer"));
    expect(screen.queryByText(/发现未完成传输/)).not.toBeInTheDocument();
  });

  // GUI-T3: 确认续传 → 只关闭弹窗（引擎已自动续传），不触发取消
  it("confirming resume only closes the dialog", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    eventMock.emit("share://progress", JSON.stringify({ type: "resume", done: 34, total: 120 }));
    await screen.findByText(/发现未完成传输/);
    fireEvent.click(screen.getByRole("button", { name: "继续续传" }));
    await waitFor(() =>
      expect(screen.queryByText(/发现未完成传输/)).not.toBeInTheDocument()
    );
    expect(invoke).not.toHaveBeenCalledWith("cancel_transfer");
  });

  // GUI-T3 修复: 续传确认弹窗 60s 无响应自动关闭（规格 §九：超时默认继续续传）。
  it("auto-closes the resume dialog after 60s without a response", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    vi.useFakeTimers();
    try {
      act(() => {
        eventMock.emit(
          "share://progress",
          JSON.stringify({ type: "resume", done: 34, total: 120 })
        );
      });
      expect(screen.getByText(/发现未完成传输/)).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(60_000);
      });
      expect(screen.queryByText(/发现未完成传输/)).not.toBeInTheDocument();
      expect(invoke).not.toHaveBeenCalledWith("cancel_transfer");
    } finally {
      vi.useRealTimers();
    }
  });

  // GUI-T3: 面板取消按钮 → cancel_transfer + 面板终态（按钮禁用）
  it("cancel button invokes cancel_transfer and ends the panel", async () => {
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    await screen.findByText(/正在连接/);
    fireEvent.click(screen.getByRole("button", { name: /取消传输/ }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("cancel_transfer"));
    expect(await screen.findByText(/传输失败或已取消/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /取消传输/ })).toBeDisabled();
  });

  // GUI-T3: 配对码大字 + 复制按钮（navigator.clipboard）+ 有效期提示
  it("copies the pairing code with feedback", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    vi.mocked(invoke).mockResolvedValue(null);
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    eventMock.emit("share://code", "8-purple-hammer");
    expect(await screen.findByText("8-purple-hammer")).toBeInTheDocument();
    expect(screen.getByText(/24 小时内有效/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /复制/ }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("8-purple-hammer"));
    expect(await screen.findByText("已复制 ✓")).toBeInTheDocument();
  });
});
