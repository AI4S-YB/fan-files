# fan-files Search、Suggest 与索引优化说明

日期：2026-08-13
PR 分支：`fix/search-suggest-production`
原始优化提交：`8e330d1 fix: bound search and suggest at production scale`

## 1. 优化背景

58 服务器当前 SQLite 数据库包含 6,499,378 个有效文件，但生产 Tantivy
索引中已有 12,654,401 个文档，约为有效文件数的 1.95 倍。旧程序在文件更新时
未能真正删除相同 `file_id` 的旧索引文档，导致索引持续重复增长。

在此数据规模下，旧版 `search` 和 `suggest` 还存在全表扫描、全量 embedding
读取及逐文件查询等问题。生产基线测试中，多个搜索请求超过 45 秒仍不能完成，
部分请求在 180 秒后仍超时；`suggest` 也可能超过 60 秒。

本次优化的目标是：

- 消除交互式查询热路径中的全表扫描和 N+1 查询；
- 将 suggest 从文件级全库检索改为 dataset 级有限候选推荐；
- 修复 Tantivy 重复文档累积；
- 提供可校验、可原子切换的完整索引重建流程；
- 保证 search、suggest 等只读操作不修改生产数据目录。

## 2. Search 搜索优化

### 优化前

- Tantivy 没有结果时，回退到 SQLite `LIKE '%关键词%'` 查询；
- 模糊查询需要扫描约 649 万条文件记录；
- 搜索可能读取完整 embeddings 表并初始化 ONNX 模型；
- 每次搜索还会执行全库元数据覆盖率统计；
- 获取结果时存在多次逐条 SQLite 查询。

### 优化后

- 交互式搜索只使用 Tantivy 全文索引，不再执行 SQLite 全表模糊扫描；
- 最多获取 50 个全文搜索候选；
- embedding 仅查询这批候选文件，不再读取完整 embeddings 表；
- embeddings 为空时不加载 ONNX 模型；
- 删除搜索热路径中的全库统计；
- 对候选进行合并排序后，仅构造排名最高的 20 条结果。

### 功能取舍

Tantivy 没有收录的内容不再通过昂贵的 SQLite 模糊查询兜底。因此，搜索结果的
完整性依赖 Tantivy 索引是否及时、完整地重建。相比让任意查询触发 649 万行全表
扫描，这一取舍更适合生产交互式查询。

## 3. Suggest 推荐优化

### 优化前

- 根据输入路径搜索大量文件；
- 可能加载全部文件路径；
- 对文件逐条查询元数据，存在 N+1 查询；
- 推荐结果偏向文件级结果，难以准确表达相关数据集；
- 查询耗时随文件总数增长。

### 优化后

新的 suggest 查询链路如下：

1. 根据输入路径定位最具体的当前 dataset；
2. 获取当前 dataset 的物种和数据类型；
3. 只从同物种 dataset 中读取有限数量候选；
4. 根据数据类型互补性、类型差异和物种置信度评分；
5. 稳定排序后返回指定数量的 dataset 建议。

当前互补关系包括：

- RNA-seq → ChIP-seq、ATAC-seq、WGBS；
- transcriptome → epigenome、genome、proteome；
- genome → transcriptome、genome annotation、epigenome；
- genome annotation → transcriptome、proteome、functional；
- scRNA-seq → scATAC-seq、CITE-seq；
- WGS → WGBS、RNA-seq、ChIP-seq。

路径匹配使用严格的目录边界判断，不再使用可能把 `_` 解释为通配符的 SQL
`LIKE`。例如 `Oryza_sativa` 不会因为下划线而错误匹配相似名称。

## 4. Tantivy 重复索引修复

### 根因

旧 schema 中 `file_id` 只有 `STORED` 属性，没有 `INDEXED` 属性。程序虽然在
更新文件前调用了 `delete_term(file_id)`，但 Tantivy 无法按未索引字段删除旧
文档。因此，每次重新扫描都会为同一个文件追加新文档。

### 修复

- 将 `file_id` 改为 `INDEXED | STORED`；
- 添加新文档前，按 `file_id` 删除旧文档；
- 保留对旧索引 schema 的只读兼容；
- 新增单元测试，验证同一 `file_id` 二次索引后只保留一条文档；
- 增加 `num_docs()`，用于重建后的精确数量校验。

完整副本重建后的 Tantivy 文档数为 6,499,378，与 SQLite 有效文件数完全一致，
删除标记为 0。

## 5. 索引重建流程优化

旧流程一次读取全部文件，并对每个文件单独查询元数据，然后直接写入现有生产
索引。数据量较大时内存、数据库查询次数和中断风险都较高。

新流程具有以下特性：

- SQLite 以只读方式打开；
- 每批读取和提交 20,000 个文件；
- 使用联表批量取得路径、格式和生物元数据，消除逐文件 N+1 查询；
- 在 `.tantivy-rebuild-<PID>` 临时目录创建全新索引；
- 完成后核对 SQLite 有效文件数、实际处理数和 Tantivy 文档数；
- 三个数量完全一致后才通过目录重命名原子切换；
- 新目录切换失败时恢复旧索引；
- 构建或校验失败时不触碰现有生产索引。

在 58 服务器的完整数据库隔离副本上，6,499,378 条索引约两分钟完成重建。

## 6. 只读安全优化

- search、suggest 和重建读取阶段使用 SQLite 只读连接；
- 只读命令不再删除 Tantivy writer lock 或 meta lock；
- 已验证只读验收期间没有在生产数据库旁生成 `index.db-wal` 或
  `index.db-shm`；
- 集成测试验证只读连接会拒绝写入操作。

## 7. 验收结果

完整生产数据库副本验收结果：

- SQLite 有效文件数：6,499,378；
- 修复前生产 Tantivy 文档数：12,654,401；
- 修复后 Tantivy 文档数：6,499,378；
- Tantivy 删除标记：0；
- PR 1 独立 `fan-core` 复测：31/31 通过，其中单元测试 6 项、集成测试
  10 项、解释器测试 11 项、LLM 解析测试 4 项；
- Tantivy 去重和 suggest 排序测试均通过；
- CLI 测试：7/7 通过；
- 58 隔离端到端测试：扫描 3 个文件后立即停止 daemon，`status`、`search`、
  `info` 均可读取 WAL 中的新数据；
- 原子重建测试：3 个 SQLite 有效文件生成 3 个唯一 Tantivy 文档，重建后查询正常；
- 最终 release 构建 SHA-256：
  `c43c9d75e3c7e9e9dfc0b535c84254fccaa274905db83b58ba84be45f66244e5`。

查询热缓存验收结果：

- `search Oryza`：约 29 ms；
- `search Glycine_max`：约 260 ms；
- `search PRJNA682952`：约 13 ms；
- `suggest`：约 13–16 ms。

重建后第一次完整 mmap 冷启动曾达到约 27.5 秒；完成首次磁盘页加载后，同一
索引的后续查询降至毫秒级。因此，生产部署后应执行一次预热查询，再评估稳定
延迟。

## 8. 当前生产状态

截至 2026-08-13：

- 修复代码已经保存在本地分支和提交中；
- release 二进制和去重索引已在隔离副本完成验证；
- 58 服务器生产二进制尚未替换；
- 58 服务器生产 Tantivy 索引尚未切换；
- 现有 4.3 GB 生产备份保持不变；
- 生产仍使用包含 12,654,401 条文档的旧 Tantivy 索引。

生产切换需要明确授权。推荐上线顺序为：安装用户级修复二进制、保留旧索引为
带时间戳的回滚目录、原子切换新索引、执行预热与 search/suggest 验收；任一验收
失败时立即恢复旧二进制和旧索引。
