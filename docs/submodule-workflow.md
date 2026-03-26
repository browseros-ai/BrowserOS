# Sup-agent Submodule Workflow

This project manages `Sup-agent` as a Git submodule inside the Browser repo.

## Canonical Setup

- Parent repo: `Shimmy-Browser`
- Submodule repo: `Sup-agent`
- Submodule path: `packages/browseros-agent/vendor/sup-agent`

## Rules To Follow

1. Treat both repos as independent:
   - Separate remotes
   - Separate branches
   - Separate commit history
2. Never use the old path `Sup-agent` at repository root.
3. Keep `.gitmodules` and local `.git/config` submodule entries aligned.

## Daily Development Flow

1. Edit inside submodule:
   - `packages/browseros-agent/vendor/sup-agent`
2. Commit and push in submodule first.
3. Return to parent repo and stage submodule pointer:
   - `git add packages/browseros-agent/vendor/sup-agent`
4. Commit and push in parent repo.

## Sync / Pull Flow

After pulling parent changes:

- `git submodule update --init --recursive`

When intentionally updating submodule to newer remote state:

- `git submodule update --remote --merge packages/browseros-agent/vendor/sup-agent`
- Then commit the updated submodule pointer in parent.

## Important Behavior

- Parent repo stores only a pinned submodule commit SHA.
- Submodule code changes do not affect parent until the parent pointer is committed.
- Pulling parent can move submodule checkout to the SHA pinned by that parent commit.

## Tests in This Monorepo

`packages/browseros-agent` runs Bun tests only for the parent tree. Test discovery ignores `**/vendor/**` so Sup-agent’s own `apps/server/tests` are not executed here (they assume Sup-agent’s standalone install and dependencies). Built-in skill **content** is still loaded from `vendor/sup-agent/.../skills/defaults` at build and runtime via `DEFAULT_SKILLS`.
