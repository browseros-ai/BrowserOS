# ACL Matcher

The ACL matcher blocks guarded tool actions (click, fill, hover, etc.) when they target elements that match user-defined access control rules. It scores each rule against the target element using a combination of exact, fuzzy, and semantic similarity — then blocks if the confidence exceeds a threshold.

## How it works

When a guarded tool is invoked, `acl-guard.ts` resolves the target element's properties (text content, aria labels, attributes, etc.) and runs them through the scoring pipeline:

1. **Site filtering** — rules are filtered to those matching the current page URL
2. **Site-only rules** — rules with no selector/text/description block the entire site immediately
3. **Element scoring** — remaining rules are scored against the element using three signals:

| Signal | Weight | How it works |
|--------|--------|-------------|
| Exact | 25% | Are any compiled rule terms a substring of an element field? |
| Fuzzy | 25% | Edit distance ratio between rule terms and element text windows |
| Semantic | 50% | Cosine similarity of sentence embeddings (BAAI/bge-small-en-v1.5 via ONNX) |

The weighted scores produce a **confidence** value between 0 and 1. If confidence >= **0.4** (Handpicked, needs updating), the action is blocked.

## Files

| File | Purpose |
|------|---------|
| `acl-guard.ts` | Entry point — called by `framework.ts` during tool execution |
| `acl-scorer.ts` | Core pipeline: text normalization, feature extraction, scoring, decision |
| `acl-embeddings.ts` | Lazy-loaded `@huggingface/transformers` pipeline for semantic similarity |
| `acl-edit-distance.ts` | Levenshtein edit distance ratio for fuzzy matching |
| `acl-stopwords.ts` | Static set of 198 English stopwords (from NLTK corpus) |

Shared types and basic matchers live in `packages/shared/`:
- `src/types/acl.ts` — `AclRule` and `ElementProperties` interfaces
- `src/acl/match.ts` — site pattern globbing and CSS selector matching

## Embedding model

The semantic scoring uses [BAAI/bge-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5) (~33MB ONNX model) via `@huggingface/transformers`. The model downloads automatically on first use and is cached for the process lifetime.

Override the model with the `ACL_EMBEDDING_MODEL` environment variable (e.g. `ACL_EMBEDDING_MODEL=Xenova/bge-base-en-v1.5`).

## Testing

```bash
bun --env-file=.env.development test apps/server/tests/tools/acl-scorer.test.ts
```

Test fixtures live in `apps/server/tests/__fixtures__/acl/` (courtesy of claude code):

| Fixture | Tests |
|---------|-------|
| `submit-button.json` | Exact match — "Place Order" button vs "block checkout submit" rule |
| `semantic-payment.json` | Semantic match — "Proceed to Checkout" vs "prevent purchase actions" |
| `semantic-delete.json` | Semantic match — "Remove my account permanently" vs "block account deletion" |
| `semantic-send-email.json` | Semantic match — send button vs "do not dispatch emails" |
| `semantic-safe.json` | False positive — "View Report" should NOT be blocked by payment/delete rules |
