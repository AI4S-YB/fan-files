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
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("write_config", expect.objectContaining({ include: [] })));
  });
  it("falls back to empty defaults when read_config fails", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("io error"));
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    expect(screen.getByLabelText("Endpoint")).toHaveValue("");
    expect(screen.getByText(/还没有添加数据目录/)).toBeInTheDocument();
  });
  it("removes a directory from the list before saving", async () => {
    vi.mocked(invoke).mockResolvedValue({ include: ["/data/x", "/data/y"], exclude: [], endpoint: "", api_key: "", model: "" });
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getAllByDisplayValue(/^\/data\//)).toHaveLength(2));
    fireEvent.click(screen.getAllByRole("button", { name: "移除" })[0]);
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("write_config", expect.objectContaining({ include: ["/data/y"] }))
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
});
