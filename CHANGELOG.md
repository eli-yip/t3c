# Changelog

## [Unreleased]

### Added

- 新增 `t3c search`：搜索 Things 3 待办标题、正文和检查清单，默认排除已完成、已取消及废纸篓内容，并跳过已勾选检查清单；支持 JSON、分页，以及分别包含这些内容的选项。
- 新增 `t3c complete <ID>`：按 ID 完成待办并验证结果，支持 JSON；重复执行不会重新设置完成时间。
- 新增 `skills/t3c` Agent 技能：仅允许用户显式调用，说明搜索与完成待办的用法，优先使用 JSON 结果。
