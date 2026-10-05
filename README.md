# t3c

在终端搜索 Things 3 待办的标题、正文和检查清单，并按 ID 标记完成。

```sh
t3c search weibo
t3c search "weekly review" --json
t3c search weibo --limit 20 --offset 20
```

## 安装

需要 macOS、Things 3 和支持 edition 2024 的 Rust 工具链。构建时需要 Xcode Command Line Tools。

在项目目录执行：

```sh
cargo install --path . --locked
```

## 搜索

关键词按连续文本匹配，英文字母大小写不敏感；`%`、`_`、`*` 都是普通字符。多个词请加引号。

默认返回全部匹配的未完成待办，排除已完成、已取消、废纸篓中的待办，以及废纸篓项目内的待办。检查清单默认只搜索未勾选项；仅命中已勾选项的待办不会出现在结果中。同一待办命中多个字段只计一次，按修改时间从新到旧排列，同一时间按 ID 排序。

| 参数 | 用途 |
| --- | --- |
| `--json` | 导出结构化结果，保留完整命中字段 |
| `--limit N` | 最多返回 N 条待办，N 大于 0 |
| `--offset N` | 跳过前 N 条匹配待办，默认 0 |
| `--include-completed` | 包含已完成待办和已勾选检查清单项 |
| `--include-canceled` | 包含已取消待办 |
| `--include-trashed` | 包含废纸篓内容 |

```sh
t3c search weibo --json > results.json
t3c search weibo --include-trashed
```

三个 `--include-*` 参数互相独立，可组合使用。

JSON 的 `total` 是分页前的匹配待办数，`count` 是本次返回数，`has_more` 表示后面还有结果。每条待办的 `matches` 标明命中的标题、正文或检查清单项。无结果时返回空列表并正常退出。

默认自动寻找本机 Things 数据库，也可以指定：

```sh
T3C_DATABASE='/path/to/main.sqlite' t3c search weibo
```

搜索只读本地数据，无需 Token 或 Shortcuts。搜索对象是待办，不包括项目自身的标题、正文或标签。依赖 Things 内部数据库结构，Things 更新后可能需要适配；目前仅在本机 macOS 的 Things 数据库上验证。

## 完成待办

复制搜索结果中的待办 ID：

```sh
t3c complete <ID>
t3c complete <ID> --json
```

确认 Things 和本地数据库中的状态已完成后才返回成功。已经完成的待办直接成功，JSON 中 `changed` 为 `false`，不重新设置完成时间。

只接受单个待办 ID；项目、检查清单项、已取消、废纸篓内容及重复规则模板不能作为目标。重复任务请使用具体实例的 ID。

首次运行可能需要在 macOS「系统设置 → 隐私与安全性 → 自动化」中允许调用方控制 Things。无需 Token；操作通过 Things 执行，不直接写数据库。`T3C_DATABASE` 只能指向本机自动发现的同一数据库。

如果提示无法确认完成状态，请先查看 Things；操作可能已经生效。

## Agent 技能

项目提供 [t3c 技能](skills/t3c/SKILL.md)，指导 Agent 搜索和完成待办，优先使用 `--json`。

先安装 t3c 并确保 Agent 能在命令行运行它，再将整个 `skills/t3c` 目录安装到所用 Agent 的技能目录，保留其中的 `agents/openai.yaml`。

技能仅允许用户显式调用，已关闭 Codex 和 Claude Code 的模型主动调用。安装后在对话中使用：

- Codex：`$t3c 搜索包含 weibo 的未完成待办`
- Claude Code：`/t3c 搜索包含 weibo 的未完成待办`

需要完成某项时，明确指定目标，例如「使用 t3c 技能，将 ID 为 abc123 的待办标记完成」。
