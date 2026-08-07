# Worktrunk Setup

This repo uses [Worktrunk](https://github.com/max-sixty/worktrunk) for running multiple Claude Code agents in parallel on different branches.

## Install Worktrunk

```bash
brew install max-sixty/worktrunk/wt
wt config shell install
# restart terminal
```

## Quick Commands

| Task | Command |
|------|---------|
| Create worktree + start Claude | `wt switch -c -x claude feat-name` |
| Switch to existing worktree | `wt switch feat-name` |
| List all worktrees | `wt list` |
| Create PR | `gh pr create` |
| Remove worktree | `wt remove feat-name` |

## What happens on `wt switch -c`

1. Creates new worktree at `../browseros-server.feat-name/`
2. Runs `bun install`
3. Copies root `.env.development` and `.env.production` files from main worktree when present

## Hooks

Hooks are configured in `.config/wt.toml`:

- **post-create**: Runs `bun install` and copies root env files from the main worktree
