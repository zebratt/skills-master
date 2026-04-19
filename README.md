# skm — SkillsMaster

跨 AI 工具的 skills 分发与升级管理器。单一事实来源 = `~/.agents/skills/`; 各工具通过 symlink（Claude Code / Codex / OpenClaw）或源读取（Hermes）消费。

## Status

Phase 1 MVP 骨架。不能用。

## Commands (Phase 1)

```
skm init                    # 初始化 ~/.agents/skills-manager/ 目录结构
skm sync [--dry-run]        # 按 frontmatter.tools 分发 symlink
skm status [--json]         # 矩阵：skill × tool × mode
skm import <path> [--as]    # plugin skill 提升到 canonical source
```

## Phase 1.5 (deferred)

`upgrade` / `doctor` / `validate` — 等 Phase 1 三命令闭环在 Matt 机器上真实工作 2 周之后。

## Build

```
cargo build --release
./target/release/skm --help
```

## License

MIT
