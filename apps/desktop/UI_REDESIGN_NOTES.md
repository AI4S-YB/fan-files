# Desktop UI Redesign — 变更说明

## 新增
- 浅/暗/跟随系统主题切换（localStorage: `fan-files-theme`）
- 设计令牌（CSS 变量）：颜色、圆角、阴影、字体
- `.btn` / `.card` / `.input` / `.badge-info|success|warning|destructive|muted` 共享工具类
- `useTheme` hook + `ThemeToggle` 组件

## 改造
- 侧边栏 180px → 200px，加 hover 高亮 + 选中左侧 3px 主色条 + 底部分隔线 + 主题切换
- 内容区 padding 20px → 24px，Header 固定顶部（页面标题 + 主题切换）
- 4 个 page 顶部重复的 `<h2>` 移除（统一在 App header 显示）
- HomePage 统计卡：auto-fit grid + 渐变顶条 + tabular-nums
- TransferPanel：8px 细进度条 + speed/ETA + role=progressbar
- SharePanel：26px 配对码 + 复制反馈（✓ 已复制）
- DatasetDetailModal：毛玻璃遮罩 + 24px 圆角 + 圆形关闭按钮
- SettingsPage 新增"外观"section 放 ThemeToggle

## 删除
- 每个 page 内部的 `<h2>`（移到 App header）

## 兼容性
- 0 个新 runtime 依赖
- 0 个新 build 依赖
- 仅 vite/vitest 现有 dev-deps 即可

## 测试
- 153+ 个测试全过
- npm run build 成功
