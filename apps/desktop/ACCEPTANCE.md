# 桌面壳 v1 验收清单

> 分支 `feat/desktop-app`。打勾状态基于自动化验证（CI/单测/冒烟/子进程级 E2E）；
> 标注 ✋ 的条目需要人工在 GUI 上目测（agent 环境受 macOS TCC 限制无法截屏/点击）。

## Mac（开发机，`npm run tauri dev` 或打包 .app）

- [ ] ✋ 首次启动：首页显示"选择目录开始扫描"空状态
- [ ] ✋ 设置页选目录 → 目录选择器弹出（async 修复后不应死锁）→ 添加进 include 列表
- [ ] ✋ 保存配置 → 页内出现"已保存 ✓"；重开应用配置仍在
- [ ] ✋ 首页点"重新扫描" → 进度行滚动（含 "Bottom-Up: NNNN directories found"）→ 完成 → 统计卡刷新
      （自动化已验证：扫描子进程管线端到端、事件流、onDone 刷新；未目测 GUI 渲染）
- [ ] ✋ 数据集页：表格/筛选/分页/详情弹层（资产+文件列表）；📂 打开目录唤起 Finder；📤 共享灰置 tooltip "即将推出"
- [ ] ✋ 搜索页：输入 "Oryza" 与中文物种名均出结果；无结果与失败各有提示
- [ ] ✋ 设置页"测试连接"：对有效/无效 endpoint+key 分别反馈 连接成功/连接失败
- [ ] ✋ 托盘：菜单三项（打开窗口/立即扫描/退出）生效；"退出"后 `ps aux | grep fan-files-share` 无残留、17951 端口释放（自动化已验证 graceful quit 清理）
- [ ] ✋ 与 CLI 共存：CLI `fan-files search xxx` 与 GUI 看到同一份索引
- [ ] （已知限制）macOS 关窗会销毁窗口，托盘"打开窗口"在窗口已关闭时无操作——记录为 v1.1 待办

## Windows（CI 产物安装包 / VM）

- [ ] ✋ 安装包安装、启动、首次流程同 Mac
- [ ] ✋ 点 X 收托盘（不退出）；托盘菜单三项生效
- [ ] ✋ 目录选择器 / 打开目录（资源管理器）正常
- [ ] CI：`build-desktop.yml` 三平台全绿（2026-08-18 已验证，含 NSIS/MSI 打包）

## 机器已验证项（无需人工重复）

| 项 | 验证方式 |
|------|------|
| share sidecar 生命周期（端口回退/健康检查/db 缺失友好错误/退出清理） | T16 实机（含 bundle 内 sidecar 解析、TCC 根因修复） |
| 扫描编排（spawn discover/stderr 流/互斥/定时循环/托盘触发） | T17 小目录 E2E + 61 前端测试 |
| share 四端点（healthz/stats/datasets/search） | T18 冒烟 bioinfo7 实测 SMOKE OK |
| 三平台构建 | T19 GitHub Actions 全绿 |
| Rust 单测 11 / 前端测试 61 | `cargo test --lib` + `npm test` |

## 已知待办（v1 后）

- macOS 关窗重建窗口 / 常驻托盘（T14 记录）
- write_config 的未知节注释保留（toml_edit 方案，T12 记录）
- check_update 在打包环境的网络路径实测
- DMG 美化步骤在本机 agent shell 失败（Finder 自动化权限），需真实终端跑一次完整 `tauri build`
- 本地 Mac db 为 schema v2（`/stats` 500），跑一次 `fan-files discover` 升级到 v4
