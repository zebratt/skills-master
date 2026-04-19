//! Distribution = realization of state.distribution onto the filesystem.
//!
//! Phase 1 implements two strategies:
//!   - Symlink  (claude / codex / openclaw): ln -s ~/.agents/skills/<name> ~/.{tool}/skills/<name>
//!   - SourceConsumer (hermes): no-op, just record mode in state
//!
//! Phase 1.5 will add Copy and Native.

// TODO Phase 1:
// - fn distribute(skill_name: &str, tool: Tool, source_root: &Path, dry_run: bool) -> SkmResult<Plan>
// - fn apply(plan: Plan) -> SkmResult<Applied>
