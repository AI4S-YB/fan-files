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

## GUI-T5 修复验收（2026-08-20，分支 feat/desktop-app @ 7525cb5）

### 修复内容（GUI-T4 审查 3 项回归 + 1 项规格遗漏）

| # | 问题 | 修复 |
|---|------|------|
| 1 | 共享完成不刷新传输历史 | 共享状态提升到页面级 `useShareTransfer`，`share://done` → `loadHistory()`（与接收完成对称） |
| 2 | 共享传输中关弹层丢跟踪 | 共享状态 + share:// 监听移到页面级，弹层改纯展示（props 注入）；弹层关闭后页面级共享面板接管进度/取消入口 |
| 3 | 搜索框 Enter 提交无 loading 防护 | `onKeyDown` Enter 时 `!loading` 才提交（按钮 disabled 拦不住键盘路径） |
| 4 | 统计卡 approximate 无标记 | `Stats.approximate` 为真时三个计数卡加 `~` 前缀 + title 提示 |

新增组件：`hooks/useShareTransfer.ts`、`components/SharePanel.tsx`、`components/ResumeDialog.tsx`（接收/共享续传弹窗统一）。

### 自动化验证

- 引擎全量（bioinfo7，rustup 1.96.1）：`cargo test -p fan-files` = **36 单测 + 7 CLI 集成全绿**；`cargo test -p fan-files-share` = **23 全绿**
- 前端全量（Mac mini Node v24）：**`npm test` 12 文件 104 用例全绿**（基线 96 + 新增 8）；`npm run build`（tsc + vite）通过
- bioinfo7 Node v18 无法跑前端测试（jsdom ESM 依赖不兼容），前端测试以 Mac mini 为准

### 双机实测（引擎级 JSONL，GUI 手动操作受 macOS TCC 限制无法自动化——见上文 ✋ 标注说明）

| 方向 | 结果 |
|------|------|
| Mac mini 发 → bioinfo7 收（码 `7-penetrate-payday-direction`） | relay 降级传输成功，SHA-256 文件级校验通过（3/3 文件哈希一致），双方 `transfer log --json` 均落审计记录 |
| bioinfo7 发 → Mac mini 收（码 `6-finicky-watchword-certify`） | relay 降级传输成功，SHA-256 校验通过（3/3 一致） |
| JSONL 事件流（FAN_JSON_PROGRESS=1，GUI 传输面板输入契约） | 双方 stdout 均输出 `{"type":"conn"}`(punching→relay) → `{"type":"progress"}`(0→100%) → `{"type":"done","ok":true,...}`，与前端 TransferPanel 解析逻辑一致 |

> 双机跨网（家庭宽带 ↔ 阿里云）UDP 打洞均失败降级 relay——与历史实测一致（ICE/CR 轮），非本次回归。
> 前端共享/接收面板 UI 交互（弹层关闭后页面级面板接管、历史自动刷新、Enter 防护、~ 标记）由 104 个前端测试覆盖。
