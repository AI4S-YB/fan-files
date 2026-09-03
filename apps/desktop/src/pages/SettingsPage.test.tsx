import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import SettingsPage from "./SettingsPage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";

beforeEach(() => {
  vi.resetAllMocks();
});

const DEFAULT_CFG = { include: [], exclude: [], endpoint: "", api_key: "", model: "" };
// 挂载时 read_transfer_config 的返回值（传输参数 [transfer] 段）
const TRANSFER_CFG = {
  chunk_size_mb: 4,
  concurrency: 4,
  receive_dir: null,
  udp_enabled: true,
};

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
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockRejectedValueOnce(new Error("disk full"));
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(screen.getByText(/保存失败/)).toBeInTheDocument());
    expect(screen.queryByText(/已保存/)).not.toBeInTheDocument();
  });
  // T13: 添加目录走原生目录选择器 pick_directory 命令
  it("invokes pick_directory when adding a directory, cancel keeps list unchanged", async () => {
    // 挂载时的 read_config / read_transfer_config，然后点击后的 pick_directory（用户取消 → null）
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce(null);
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: /添加目录/ }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("pick_directory"));
    // 用户取消（null）：include 列表保持不变——空列表提示仍在，且没有目录输入行
    expect(screen.getByText(/还没有添加数据目录/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/目录 \d+/)).not.toBeInTheDocument();
  });
  it("appends the picked directory to the include list", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce("/a");
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: /添加目录/ }));
    // include 列表出现新目录输入项
    await waitFor(() => expect(screen.getByDisplayValue("/a")).toBeInTheDocument());
  });
  // T13: 测试连接 → test_connection 命令，按返回值展示 连接成功/连接失败
  it("shows 连接成功 when test_connection returns true", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce(true);
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
    await waitFor(() => expect(screen.getByText(/连接成功/)).toBeInTheDocument());
    expect(invoke).toHaveBeenCalledWith(
      "test_connection",
      expect.objectContaining({ cfg: expect.objectContaining({ endpoint: "" }) })
    );
  });
  it("shows 连接失败 when test_connection returns false", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce(false);
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
    await waitFor(() => expect(screen.getByText(/连接失败/)).toBeInTheDocument());
    expect(screen.queryByText(/连接成功/)).not.toBeInTheDocument();
  });
  // T13: 检查更新 → check_update 命令，反馈行显示返回文本
  it("shows check_update output text", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce("已是最新版本 v1.2.3");
    render(<SettingsPage />);
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await waitFor(() => expect(screen.getByText(/已是最新版本 v1\.2\.3/)).toBeInTheDocument());
  });

  // GUI-T3: 传输设置区渲染（块大小/并发/UDP 直连默认值 + 保存按钮）
  it("renders the transfer settings section with defaults", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(DEFAULT_CFG).mockResolvedValueOnce(TRANSFER_CFG);
    render(<SettingsPage />);
    const chunkSelect = await screen.findByLabelText("块大小（MB）");
    expect(chunkSelect).toHaveValue("4");
    expect(screen.getByLabelText("并发数（同时传输的块数）")).toHaveValue("4");
    expect(screen.getByLabelText(/启用 UDP 直连/)).toBeChecked();
    expect(screen.getByRole("button", { name: "保存传输设置" })).toBeInTheDocument();
    // 未设置默认接收目录 → 提示默认路径
    expect(screen.getByDisplayValue(/未设置（默认 ~\/Downloads\/fan-received）/)).toBeInTheDocument();
  });

  // GUI-T3: 保存传输设置 → write_transfer_config（块大小/并发/UDP 透传）
  it("saves transfer config via write_transfer_config on button click", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValue(undefined); // 后续 write_transfer_config
    render(<SettingsPage />);
    fireEvent.change(await screen.findByLabelText("块大小（MB）"), {
      target: { value: "8" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存传输设置" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "write_transfer_config",
        expect.objectContaining({
          cfg: expect.objectContaining({
            chunk_size_mb: 8,
            concurrency: 4,
            udp_enabled: true,
            receive_dir: null,
          }),
        })
      )
    );
    expect(await screen.findByText(/已保存/)).toBeInTheDocument();
  });

  // GUI-T3: UDP 直连 toggle 关闭后保存 → udp_enabled=false
  it("saves udp_enabled=false when the toggle is off", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValue(undefined); // 后续 write_transfer_config
    render(<SettingsPage />);
    fireEvent.click(await screen.findByLabelText(/启用 UDP 直连/));
    fireEvent.click(screen.getByRole("button", { name: "保存传输设置" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "write_transfer_config",
        expect.objectContaining({
          cfg: expect.objectContaining({ udp_enabled: false }),
        })
      )
    );
  });

  // GUI-T3: 默认接收目录走原生目录选择器 pick_directory
  it("picks the default receive directory via pick_directory", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce("/data/received");
    render(<SettingsPage />);
    fireEvent.click(await screen.findByRole("button", { name: /选择目录/ }));
    await waitFor(() => expect(screen.getByDisplayValue("/data/received")).toBeInTheDocument());
  });

  // SF-T4: 从 CC Switch 接管——1 个 profile → 直接 read_cc_switch_profile 填充表单，不弹窗
  it("fills directly from a single CC Switch profile without opening the dialog", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce([
        { name: "haikou-flash", api_type: "anthropic", model: "claude-sonnet-4-8" },
      ])
      .mockResolvedValueOnce({
        api_type: "anthropic",
        base_url: "http://10.33.105.218:3200",
        api_key: "sk-cc",
        model: "claude-sonnet-4-8",
      });
    render(<SettingsPage />);
    fireEvent.click(await screen.findByRole("button", { name: "从 CC Switch 接管" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("read_cc_switch_profile", { name: "haikou-flash" })
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Endpoint")).toHaveValue("http://10.33.105.218:3200");
    expect(screen.getByLabelText("API Key")).toHaveValue("sk-cc");
    expect(screen.getByLabelText("模型名称")).toHaveValue("claude-sonnet-4-8");
    expect(screen.getByLabelText("API 类型")).toHaveValue("anthropic");
    expect(screen.getByText(/已从 CC Switch 接管：haikou-flash/)).toBeInTheDocument();
  });

  // SF-T4: 无 profile → 提示"未找到 CC Switch 配置"，不弹窗、不读取
  it("shows 未找到 CC Switch 配置 when no profiles and does not open the dialog", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce([]);
    render(<SettingsPage />);
    fireEvent.click(await screen.findByRole("button", { name: "从 CC Switch 接管" }));
    await waitFor(() =>
      expect(screen.getByText(/未找到 CC Switch 配置/)).toBeInTheDocument()
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith("read_cc_switch_profile", expect.anything());
    expect(screen.getByLabelText("Endpoint")).toHaveValue("");
  });

  // SF-T4: 多个 profile → 弹窗列出（名称 + 协议徽标 + 模型名），选中后填充
  it("lists multiple profiles in the picker dialog and fills the form on selection", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce([
        { name: "haikou-flash", api_type: "anthropic", model: "claude-sonnet-4-8" },
        { name: "official-pro", api_type: "openai", model: "deepseek-chat" },
      ])
      .mockResolvedValueOnce({
        api_type: "openai",
        base_url: "https://api.deepseek.com/v1",
        api_key: "sk-official",
        model: "deepseek-chat",
      });
    render(<SettingsPage />);
    fireEvent.click(await screen.findByRole("button", { name: "从 CC Switch 接管" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("list_cc_switch_profiles"));
    // 弹窗内两个 profile：名称 + 协议徽标 + 模型名
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("haikou-flash")).toBeInTheDocument();
    expect(within(dialog).getByText("Anthropic")).toBeInTheDocument();
    expect(within(dialog).getByText("claude-sonnet-4-8")).toBeInTheDocument();
    expect(within(dialog).getByText("official-pro")).toBeInTheDocument();
    expect(within(dialog).getByText("OpenAI")).toBeInTheDocument();
    expect(within(dialog).getByText("deepseek-chat")).toBeInTheDocument();
    // 点选 official-pro → read_cc_switch_profile("official-pro") → 表单填充 + 来源提示
    fireEvent.click(within(dialog).getByRole("button", { name: /official-pro/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("read_cc_switch_profile", { name: "official-pro" })
    );
    expect(screen.getByLabelText("Endpoint")).toHaveValue("https://api.deepseek.com/v1");
    expect(screen.getByLabelText("API Key")).toHaveValue("sk-official");
    expect(screen.getByLabelText("模型名称")).toHaveValue("deepseek-chat");
    expect(screen.getByLabelText("API 类型")).toHaveValue("openai");
    expect(screen.getByText(/已从 CC Switch 接管：official-pro/)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  // SF-T4: 弹窗取消 → 不填充表单、不调用读取
  it("closes the picker without filling the form on cancel", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValueOnce([
        { name: "haikou-flash", api_type: "anthropic", model: "claude-sonnet-4-8" },
        { name: "official-pro", api_type: "openai", model: "deepseek-chat" },
      ]);
    render(<SettingsPage />);
    fireEvent.click(await screen.findByRole("button", { name: "从 CC Switch 接管" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(invoke).not.toHaveBeenCalledWith("read_cc_switch_profile", expect.anything());
    expect(screen.getByLabelText("Endpoint")).toHaveValue("");
    expect(screen.queryByText(/已从 CC Switch 接管/)).not.toBeInTheDocument();
  });

  // SF-T4: list 命令本身失败（如二进制缺失）→ 错误展示，不弹窗
  it("shows the list error when list_cc_switch_profiles fails", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(DEFAULT_CFG)
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockRejectedValueOnce(new Error("读取 CC Switch 配置失败"));
    render(<SettingsPage />);
    fireEvent.click(await screen.findByRole("button", { name: "从 CC Switch 接管" }));
    await waitFor(() =>
      expect(screen.getByText(/读取 CC Switch 配置失败/)).toBeInTheDocument()
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  // NR-T4: API 类型下拉切换后保存 → write_config 的 cfg 含 api_type
  it("saves api_type with the config when the dropdown changes", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce({ ...DEFAULT_CFG, api_type: "openai" })
      .mockResolvedValueOnce(TRANSFER_CFG)
      .mockResolvedValue(undefined);
    render(<SettingsPage />);
    const select = await screen.findByLabelText("API 类型");
    expect(select).toHaveValue("openai");
    fireEvent.change(select, { target: { value: "anthropic" } });
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "write_config",
        expect.objectContaining({
          cfg: expect.objectContaining({ api_type: "anthropic" }),
        })
      )
    );
  });

  // NR-T4: 账号与崖州湾试用占位区已删除
  it("does not render the account/placeholder section", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(DEFAULT_CFG).mockResolvedValueOnce(TRANSFER_CFG);
    render(<SettingsPage />);
    await screen.findByRole("button", { name: "保存配置" });
    expect(screen.queryByText(/账号与崖州湾试用/)).not.toBeInTheDocument();
    expect(screen.queryByText(/即将上线/)).not.toBeInTheDocument();
  });
});
