import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SettingsPage from "./SettingsPage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";

beforeEach(() => {
  vi.resetAllMocks();
});

const DEFAULT_CFG = { include: [], exclude: [], endpoint: "", api_key: "", model: "" };

describe("SettingsPage", () => {
  it("loads config into fields", async () => {
    vi.mocked(invoke).mockResolvedValue({ include: ["/data/x"], exclude: [], endpoint: "http://e", api_key: "k", model: "m" });
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByDisplayValue("/data/x")).toBeInTheDocument());
  });
  it("saves config on button click", async () => {
    vi.mocked(invoke).mockResolvedValue({ include: [], exclude: [], endpoint: "", api_key: "", model: "" });
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "write_config",
        expect.objectContaining({ cfg: expect.objectContaining({ include: [] }) })
      )
    );
  });
  it("falls back to empty defaults when read_config fails", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("io error"));
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    expect(screen.getByLabelText("Endpoint")).toHaveValue("");
    expect(screen.getByText(/还没有添加数据目录/)).toBeInTheDocument();
  });
  it("removes a directory from the list before saving, keeps threads passthrough", async () => {
    vi.mocked(invoke).mockResolvedValue({ threads: 8, include: ["/data/x", "/data/y"], exclude: [], endpoint: "", api_key: "", model: "" });
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getAllByDisplayValue(/^\/data\//)).toHaveLength(2));
    fireEvent.click(screen.getAllByRole("button", { name: "移除" })[0]);
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "write_config",
        expect.objectContaining({ cfg: expect.objectContaining({ include: ["/data/y"], threads: 8 }) })
      )
    );
  });
  it("shows in-page feedback after a successful save", async () => {
    vi.mocked(invoke).mockResolvedValue(DEFAULT_CFG);
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(screen.getByText(/已保存/)).toBeInTheDocument());
    expect(screen.queryByText(/保存失败/)).not.toBeInTheDocument();
  });
  it("shows error line when write_config fails", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockRejectedValueOnce(new Error("disk full"));
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(screen.getByText(/保存失败/)).toBeInTheDocument());
    expect(screen.queryByText(/已保存/)).not.toBeInTheDocument();
  });
  // T13: 添加目录走原生目录选择器 pick_directory 命令
  it("invokes pick_directory when adding a directory", async () => {
    // 第一次调用是挂载时的 read_config，第二次是点击后的 pick_directory（用户取消 → null）
    vi.mocked(invoke).mockResolvedValueOnce(DEFAULT_CFG).mockResolvedValueOnce(null);
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: /添加目录/ }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("pick_directory"));
  });
  it("appends the picked directory to the include list", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(DEFAULT_CFG).mockResolvedValueOnce("/a");
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: /添加目录/ }));
    // include 列表出现新目录输入项
    await waitFor(() => expect(screen.getByDisplayValue("/a")).toBeInTheDocument());
  });
  // T13: 测试连接 → test_connection 命令，按返回值展示 连接成功/连接失败
  it("shows 连接成功 when test_connection returns true", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(DEFAULT_CFG).mockResolvedValueOnce(true);
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
    await waitFor(() => expect(screen.getByText(/连接成功/)).toBeInTheDocument());
    expect(invoke).toHaveBeenCalledWith(
      "test_connection",
      expect.objectContaining({ cfg: expect.objectContaining({ endpoint: "" }) })
    );
  });
  it("shows 连接失败 when test_connection returns false", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(DEFAULT_CFG).mockResolvedValueOnce(false);
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
    await waitFor(() => expect(screen.getByText(/连接失败/)).toBeInTheDocument());
    expect(screen.queryByText(/连接成功/)).not.toBeInTheDocument();
  });
  // T13: 检查更新 → check_update 命令，反馈行显示返回文本
  it("shows check_update output text", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce("已是最新版本 v1.2.3");
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await waitFor(() => expect(screen.getByText(/已是最新版本 v1\.2\.3/)).toBeInTheDocument());
  });
});
