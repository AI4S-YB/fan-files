# fan-files 易用性改进

## 概述

基于测试反馈，改进三个核心体验缺口：首次配置引导、daemon+infer 自动化、搜索体验。

## 1. `fan-files init` 交互式初始化

```
$ fan-files init

  ╔══════════════════════════════════════╗
  ║   Fan-Files 初始化配置向导          ║
  ╚══════════════════════════════════════╝

  ▸ 步骤 1/3：扫描目录

  请输入要扫描的目录（绝对路径）:
  [1] /home/kentnf (当前用户家目录)
  [2] 自定义路径
  请输入序号或路径: _

  已添加: /home/kentnf/data
  是否继续添加目录？
  [1] 继续添加
  [2] 完成，进入下一步
  请输入: _

  ▸ 步骤 2/3：LLM 元数据推断

  LLM 可自动识别项目、物种、实验类型。请选择:
  [1] DeepSeek (国内推荐，性价比最高)
      模型: deepseek-chat, ~2元/百万tokens
  [2] 通义千问 Qwen (阿里云)
      模型: qwen-plus
  [3] 智谱 GLM (国产均衡)
      模型: glm-4-flash
  [4] 百度文心 ERNIE
      模型: ernie-4.0-turbo
  [5] OpenAI / 自定义 API
  [6] 暂时跳过
  请输入: _

  ▸ 步骤 2.1：API Key
  endpoint: https://api.deepseek.com/v1/chat/completions
  api_key: sk-________________
  model: deepseek-chat
  测试连接... ✅ 连接成功

  ▸ 步骤 3/3：开始扫描

  配置已保存到 ~/.fan-files/config.toml
  是否现在开始扫描和推断？
  [1] 后台运行（推荐，可以继续做其他事情）
  [2] 前台运行（显示实时进度）
  [3] 稍后手动运行
  请输入: _

  [1] 已启动后台扫描...
  进度: ████████░░░░░░░░  52% (438/846 文件)
  推断: 等待扫描完成...

  按 Ctrl+D 返回终端（扫描继续在后台运行）
  按 Ctrl+C 停止扫描
```

## 2. Daemon 自动触发 Infer

Daemon 已有代码，改为：首次扫描完自动 infer，之后增量变化 >10 个文件时重新 infer。

状态提示：

```
fan-files status
  Indexed: 846 files
  Metadata: 88% complete (745/846)
  3 projects pending review → run 'fan-files pending'
```

## 3. 搜索优化

搜索时若元数据覆盖 < 50%，结果附带提示：

```
fan-files search "RNA-seq"
  找到 5 条结果
  ⚠ 元数据覆盖较低 (当前 12%)，运行 'fan-files infer' 可获得更准确的结果
```

## 4. LLM 厂商覆盖

| 厂商 | 模型 | endpoint |
|------|------|----------|
| DeepSeek | deepseek-chat | `https://api.deepseek.com/v1/chat/completions` |
| 通义千问 (Qwen) | qwen-plus | `https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions` |
| 智谱 GLM | glm-4-flash | `https://open.bigmodel.cn/api/paas/v4/chat/completions` |
| 百度文心 (ERNIE) | ernie-4.0-turbo-8k | `https://aip.baidubce.com/rpc/2.0/ai_custom/v1/wenxinworkshop/chat/completions` |
| OpenAI / 自定义 | 用户自填 | 用户自填 |

## 5. 后台运行机制

Daemon 和 Infer 支持 `--background` 参数：

```
fan-files daemon --background     → 后台扫描+监控
fan-files infer --background      → 后台推断
fan-files daemon --foreground     → 前台显示实时进度（默认）
```

后台模式行为：
- Terminal 显示启动信息后立即返回
- 进度写入 `~/.fan-files/progress.json`
- 完成时通过 terminal bell 或 stdout 提醒
- 用户可随时 `fan-files status` 查看进度
