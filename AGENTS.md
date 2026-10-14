# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd onboard` to get started.

## Architecture

**Read [`docs/architecture.md`](docs/architecture.md) for the full reference.** Key rules:

- **`lib/`** = pure logic. No system signatures, no ECS queries. Methods take plain args, return plain results. Testable without Bevy.
- **`app/`** = Bevy integration. Systems are thin wrappers: read resources → call `lib/` functions → write back.
- **`bin/`** = standalone tools. Import from `lib/` public API only, never from `app/`.
- **No UI state in domain structs.** Interaction state (drag, hover, selection) lives in `UiState`.
- **One authoritative resource per domain.** Everything else is a mirror (e.g. `UiState.playback` mirrors `Timeline` + `SequenceManager`).
- **Extract pure functions.** If a system has >20 lines of logic, pull it into a testable `lib/` function.
- **File size signals.** Systems >200 lines, modules >500 lines (excl. tests) → split.

## Starting a Session

**Always start here:**

```bash
bd show seaview-437   # Load session context — architecture, state, what to do next
bd ready              # See available work
```

Issue **seaview-437** is the living session context. It is always open (P0) and contains the full project state, architecture overview, open issues, and handoff notes from the last session. **Read it first.**

## `!lc` Command


When the user says **`!lc`**, check issue seaview-437:

```bash
bd show seaview-437
```

## `!lcc` Command

`!lcc` means, load context and continue: !lc + continue

## `!sc` Command

When the user says **`!sc`**, update seaview-437's notes with the current session state:

```bash
bd update seaview-437 --notes "## Session Context (DATE)
### Branch: ...
### What was done ...
### Architecture ...
### What needs to happen next ...
### Open issues ...
### Test status ..."
```



## Reading Dependency Source

Git deps in `Cargo.toml` are the source of truth. `vendor/` is gitignored — never commit it.

**To browse/grep a dep's source:**
```bash
git clone <repo-url> vendor/<crate>
# then grep/read freely
```

**To patch/hack a dep locally:**
1. Clone to `vendor/<crate>` as above
2. Add a `[patch]` block to root `Cargo.toml`:
   ```toml
   [patch."https://github.com/owner/repo.git"]
   crate-name = { path = "vendor/crate-name" }
   ```
3. Revert the `[patch]` block before committing — never commit it

Note: `cargo vendor` has no selective/per-crate mode. Don't use it here.

## Quick Reference

```bash
bd show seaview-437   # Session context (read first!)
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --status in_progress  # Claim work
bd close <id>         # Complete work
bd sync               # Sync with git
```

## Test Watch

A `cargo watch` test runner logs results to `tmp/tests.log` (overwritten each run). Start it in a spare terminal:

```bash
./scripts/test-watch.sh
```

Before committing or when diagnosing failures, check the latest results:

```bash
cat tmp/tests.log        # full output
tail -20 tmp/tests.log   # summary
```

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Check test watch** - `tail -20 tmp/tests.log` — confirm no failures
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds

## Merging Branches

When the user says **"merge"**, they mean **rebase to main** (not a merge commit). Use this workflow:

```bash
git checkout main
git pull
git rebase <feature-branch>
git push
```

This keeps the history linear. Never use `git merge` unless explicitly asked for a merge commit.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:46cd31e7 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   bd dolt push
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
