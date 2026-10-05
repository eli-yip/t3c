---
name: t3c
disable-model-invocation: true
description: 使用 t3c 在 macOS 上搜索 Things 3 待办的标题、正文和检查清单，并按 ID 标记完成。适用于查找待办或完成指定待办的请求。
---

# t3c

优先使用 `--json` 获取结构化结果；不要解析面向人类的摘要。命令不可用时说明缺少 t3c，不擅自安装。参数不确定时阅读 `t3c <subcommand> --help`。

## 搜索

```sh
t3c search 'weibo' --json
t3c search 'weekly review' --json --limit 20 --offset 0
```

- 按连续文本匹配标题、正文和检查清单，ASCII 大小写不敏感。多词查询加引号，`%`、`_`、`*` 都是普通字符。
- 默认搜索未完成待办及未勾选检查清单，排除已取消和废纸篓内容；不搜索项目自身的标题、正文或标签。
- 按需添加互相独立的选项：`--include-completed` 包含已完成待办及已勾选检查清单；`--include-canceled` 包含已取消待办；`--include-trashed` 包含废纸篓内容。
- 结果按待办去重，以修改时间新到旧、ID 升序排列。`items[].id` 是待办 ID；`matches[].id` 若存在则是检查清单项 ID，不能用于完成待办。
- `matches` 提供命中字段及完整文本。`total` 是分页前匹配待办数，`count` 是本次返回数，`has_more` 表示还有结果。默认不限制条数；需要全部分页结果时持续读取至 `has_more: false`，不要把第一页描述成全部结果。
- 无匹配是成功，返回空列表。JSON 仅用于成功结果；错误是 stderr 文字，先检查退出码再解析 stdout。

## 完成待办

用户要求完成目标后，使用搜索结果中的完整待办 ID：

```sh
t3c complete '<ID>' --json
t3c complete 'ID1' 'ID2' 'ID3' --json
```

若同名结果有多条且上下文无法确定目标，先让用户选择；不要按标题猜测 ID，不要自动把搜索命中全量转成完成操作；只有用户明确授权的具体 ID 才能传给 `complete`。

- 单 ID 成功返回 `id`、`title`、`status: "completed"` 和 `changed`。`changed: false` 表示目标已经完成，不是失败。
- 命令会等待完成状态验证，不需要额外添加等待选项。
- 支持一个或多个完整待办 ID，按首次出现顺序去重、逐条完成并验证，失败后继续；不支持项目、检查清单项、已取消、废纸篓内容或重复规则模板。重复任务使用具体实例 ID。
- 多 ID 输入始终使用批量 JSON，即使去重后只剩一项。`results` 按处理顺序包含每个 ID：`outcome: "succeeded"` 附带单 ID 成功字段；`"failed"` 或 `"unconfirmed"` 附带 `id` 和 `error`。`summary.succeeded`、`summary.failed`、`summary.unconfirmed` 按去重后的 ID 汇总；已完成项也是成功。
- 全部成功退出码为 `0`；存在失败或未确认项为 `1`。批量 `--json` 在退出码 `1` 时仍须解析 stdout，逐项汇报成功、失败及未确认 ID，不能只看退出码宣称全部失败。单 ID 运行错误仍仅在 stderr 输出文字，stdout 为空；参数错误为退出码 `2`，不返回结果 JSON。
- 部分成功不会回滚。不要重新提交成功项，也不要重跑整个批次；失败项先处理具体原因，未确认项先查询状态。不要解析 `error` 文字来决定结果类别，使用 `outcome`。
- 超时后命令会只读回查同一 ID 的 Things 和数据库状态，不重试写入；恢复验证成功的 `changed: true` 表示观察到状态变化，不能归因到某个并发操作者。
- 超时或提示无法确认时，操作可能已生效；先用 `search --include-completed --json` 查找并核对同一 ID，不盲目重试或宣称失败。若仍无法确定，说明状态未确认。

搜索只读本地数据库，完成操作通过 Things 执行；无需 Token 或 Shortcuts。自动化权限被拒绝时，提示用户在 macOS「系统设置 → 隐私与安全性 → 自动化」中允许调用方控制 Things。

默认自动寻找数据库。需要指定文件时使用 `T3C_DATABASE='/path/to/main.sqlite'`；`complete` 只接受本机自动发现的同一数据库，不接受副本。
