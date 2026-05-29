# Fan-Files

智能文件元数据检索引擎——让 Claude Code 拥有全服务器视野。

## 是什么

部署在生物信息学服务器上的后台服务。自动扫描服务器所有文件，通过 LLM 推断生物学业务上下文（实验类型、物种、数据项目），建立多层索引。Claude Code 能以自然语言检索文件、发现数据项目、找到可协同分析的相关数据。

**核心场景：**
- "服务器上有什么数据？"
- "有没有苹果的 RNA-seq 数据？"
- "SMT2024 项目的物种是什么？"
- "列出所有参考基因组"

## 安装

### 1. 编译二进制

```bash
git clone git@github.com:AI4S-YB/fan-files.git
cd fan-files
cargo build --release
sudo cp target/release/fan-files /usr/local/bin/
```

### 2. 配置

编辑 `~/.fan-files/config.toml`：

```toml
[scan]
include = ["/data"]
exclude = ["/tmp", "*.tmp"]

[watch]
include = ["/data"]

# LLM 推理配置（DeepSeek / OpenAI / 兼容接口）
[llm]
endpoint = "https://api.deepseek.com/v1/chat/completions"
api_key = "sk-你的key"
model = "deepseek-chat"
```

### 3. 扫描 + 推理

```bash
fan-files daemon    # 扫描所有文件
fan-files infer     # LLM 推断项目、物种、实验类型
```

## 使用

```bash
fan-files search "apple RNA-seq"      # 自然语言搜索
fan-files projects                    # 列出所有数据项目
fan-files projects SMT2024_genome     # 查看项目详情
fan-files info /path/to/file.bam      # 查看文件元数据
fan-files suggest /data/projects/xxx  # 数据推荐
fan-files status                      # 索引状态
```

## 安装 Claude Code Skill

### 方式一：FAN Marketplace 安装（推荐）

```bash
# 安装 fan-cli（一次性）
git clone https://github.com/AI4S-YB/fan-marketplace.git
cd fan-marketplace && npm install && npm link

# 通过 Marketplace 安装 skill
fan install fan-files

# 升级
fan update fan-files
```

### 方式二：全局插件

```bash
ln -s /path/to/fan-files ~/.claude/plugins/fan-files
# 重启 Claude Code
```

### 方式三：项目级 Skill

```bash
mkdir -p .claude/skills
cp SKILL.md .claude/skills/fan-files.md
```

## 技术栈

Rust · SQLite · Tantivy · Candle (ONNX) · wasmtime · notify

## 协议

MIT
