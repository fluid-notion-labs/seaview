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



## Quick Reference

```bash
bd show seaview-437   # Session context (read first!)
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --status in_progress  # Claim work
bd close <id>         # Complete work
bd sync               # Sync with git
```

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds:
   ```bash
   cargo build --release  # ALWAYS run release build before finishing
   cargo test             # Run tests if applicable
   ```
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
