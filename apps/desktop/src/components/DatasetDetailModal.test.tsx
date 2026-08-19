import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import DatasetDetailModal from "./DatasetDetailModal";
import type { DatasetDetail, FileSummary } from "../api";
import type { ShareState } from "../hooks/useShareTransfer";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";

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

// GUI-T5 修复：共享状态与 share:// 监听已提升到页面级（useShareTransfer），
// 弹层为纯展示组件——共享状态/事件/回调全部由 props 注入，测试直接构造 props。
const onClose = vi.fn();
const onShareStart = vi.fn();
const onShareCancel = vi.fn();

function renderModal(overrides: {
  detail?: DatasetDetail;
  files?: FileSummary[];
  share?: ShareState;
  shareEvents?: never[];
  shareRaw?: string[];
} = {}) {
  render(
    <DatasetDetailModal
      detail={overrides.detail ?? detailFixture}
      files={overrides.files ?? filesFixture}
      onClose={onClose}
      share={overrides.share ?? { status: "idle" }}
      shareEvents={overrides.shareEvents ?? []}
      shareRaw={overrides.shareRaw ?? []}
      shareName="Oryza_sativa_v1"
      onShareStart={onShareStart}
      onShareCancel={onShareCancel}
    />
  );
}

beforeEach(() => {
  vi.resetAllMocks();
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
    renderModal();
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

  it("disables 共享 while a share is running (one transfer at a time)", () => {
    renderModal({ share: { status: "running" } });
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

  // GUI-T5: 共享由页面级状态驱动——弹层只转发起点（onShareStart），不自行 spawn
  it("starts a share via onShareStart with the dataset path", () => {
    renderModal();
    fireEvent.click(screen.getByRole("button", { name: /共享/ }));
    expect(onShareStart).toHaveBeenCalledWith("/data/orders/rice");
  });

  // GUI-T5: 弹层内的共享面板由页面级状态注入（share://code 等事件已在页面监听）
  it("renders the share panel with pairing code from page-level state", () => {
    renderModal({ share: { status: "code", code: "8-purple-hammer" } });
    expect(screen.getByText("8-purple-hammer")).toBeInTheDocument();
    expect(screen.getByText(/7 天内有效/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /取消传输/ })).toBeInTheDocument();
  });

  it("forwards panel cancel to onShareCancel", () => {
    renderModal({ share: { status: "running" } });
    fireEvent.click(screen.getByRole("button", { name: /取消传输/ }));
    expect(onShareCancel).toHaveBeenCalledTimes(1);
  });
});
