# t3c

在终端搜索 Things 3 待办的标题、正文和检查清单。

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

默认返回全部匹配的未完成及已取消待办，也搜索其中已勾选的检查清单；已完成待办默认排除；废纸篓中的待办及项目内的待办默认排除。同一待办命中多个字段只计一次，按修改时间从新到旧排列，同一时间按 ID 排序。

| 参数 | 用途 |
| --- | --- |
| `--json` | 导出结构化结果，保留完整命中字段 |
| `--limit N` | 最多返回 N 条待办，N 大于 0 |
| `--offset N` | 跳过前 N 条匹配待办，默认 0 |
| `--include-completed` | 包含已完成待办 |
| `--include-trashed` | 包含废纸篓内容 |

```sh
t3c search weibo --json > results.json
t3c search weibo --include-trashed
```

JSON 的 `total` 是分页前的匹配待办数，`count` 是本次返回数，`has_more` 表示后面还有结果。每条待办的 `matches` 标明命中的标题、正文或检查清单项。无结果时返回空列表并正常退出。

默认自动寻找本机 Things 数据库，也可以指定：

```sh
T3C_DATABASE='/path/to/main.sqlite' t3c search weibo
```

只读本地数据，无需 Token 或 Shortcuts。搜索对象是待办，不包括项目自身的标题、正文或标签。依赖 Things 内部数据库结构，Things 更新后可能需要适配；目前仅在本机 macOS 的 Things 数据库上验证。
