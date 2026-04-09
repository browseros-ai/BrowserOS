# ACL Lab

Python prototype harness for ACL matching outside the Bun server runtime.

## Setup

From `packages/browseros-agent/python/acl_lab`:

```bash
uv sync
```

`sentence-transformers` is a required dependency.

## Test with the fixture

> The first run wuold download the embedding model and be slightly slower

Run the Python test for the fixture:

```bash
uv run pytest -s tests
```

## Run with the server

The server-side bridge is in `apps/server/src/tools/acl-python-client.ts`.

From `packages/browseros-agent`:

```bash
BROWSEROS_ACL_PYTHON=1 bun run dev:watch
```

Current behavior:

- If `BROWSEROS_ACL_PYTHON` is unset, the server uses the existing TypeScript matcher.
- If `BROWSEROS_ACL_PYTHON=1`, the server resolves the page and element in TypeScript, then sends `{toolName, pageUrl, element, rules}` to `python -m acl_lab.rpc`.
- If the Python worker errors while enabled, the current behavior is fail-open: the action is allowed without a TypeScript fallback.

## Model

Override the default sentence-transformer model with:

```bash
ACL_LAB_EMBEDDING_MODEL=sentence-transformers/all-MiniLM-L6-v2 \
  uv run python -m acl_lab score fixtures/submit_button.json --pretty
```
