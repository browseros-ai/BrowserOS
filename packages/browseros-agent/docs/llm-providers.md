# LLM Providers

BrowserOS Agent is bring-your-own-key (BYOK): you connect your own LLM provider
account, and requests go directly from BrowserOS to that provider. Provider
selection and credentials are configured in the extension under
**AI Settings → Add Provider**.

Each built-in provider ships a template (default base URL + default model) so
setup is one click plus an API key. The source of truth for these templates is
`apps/agent/lib/llm-providers/providerTemplates.ts`; the provider factories that
turn a config into a model live in
`apps/server/src/lib/clients/llm/provider.ts` (server) and
`apps/server/src/agent/provider-factory.ts` (agent).

## Supported providers

| Provider | `id` | Default base URL | Default model |
|----------|------|------------------|---------------|
| Anthropic | `anthropic` | `https://api.anthropic.com/v1` | `claude-sonnet-4-6` |
| OpenAI | `openai` | `https://api.openai.com/v1` | `gpt-5` |
| ChatGPT Plus/Pro | `chatgpt-pro` | `https://chatgpt.com/backend-api` | `gpt-5.3-codex` |
| GitHub Copilot | `github-copilot` | `https://api.githubcopilot.com` | `gpt-5-mini` |
| Google Gemini | `google` | `https://generativelanguage.googleapis.com/v1beta` | `gemini-2.5-flash` |
| OpenRouter | `openrouter` | `https://openrouter.ai/api/v1` | `anthropic/claude-sonnet-4.5` |
| Moonshot AI | `moonshot` | `https://api.moonshot.ai/v1` | `kimi-k2.5` |
| **z.ai (GLM)** | `zai` | `https://api.z.ai/api/coding/paas/v4` | `glm-4.6` |
| Qwen Code | `qwen-code` | `https://portal.qwen.ai/v1` | `coder-model` |
| Azure OpenAI | `azure` | _(resource-specific)_ | _(deployment)_ |
| AWS Bedrock | `bedrock` | _(region-specific)_ | `anthropic.claude-sonnet-4-6` |
| Ollama (local) | `ollama` | `http://localhost:11434/v1` | `llama3.2` |
| LM Studio (local) | `lmstudio` | `http://localhost:1234/v1` | `openai/gpt-oss-20b` |
| OpenAI Compatible | `openai-compatible` | _(your endpoint)_ | _(your model)_ |

## z.ai (GLM)

[z.ai](https://z.ai) exposes the GLM models through an OpenAI-compatible API, so
it plugs in via the same `createOpenAICompatible` path as Moonshot and Ollama.

1. Create an API key at <https://z.ai/manage-apikey/apikey-list>.
2. In **AI Settings → Add Provider**, pick **z.ai**.
3. Paste the API key. The base URL defaults to
   `https://api.z.ai/api/coding/paas/v4` — override it only if you use a proxy.
4. Choose a model (default `glm-4.6`).

**Available models**

| Model | Context window | Vision |
|-------|----------------|--------|
| `glm-4.6` _(default)_ | 200K | yes |
| `glm-4.5` | 200K | yes |
| `glm-4.5-air` | 128K | yes |

**Config shape** (SDK / `LLMConfig`):

```jsonc
{
  "provider": "zai",
  "model": "glm-4.6",
  "apiKey": "<your z.ai key>",
  // optional — defaults to EXTERNAL_URLS.ZAI_API
  "baseUrl": "https://api.z.ai/api/coding/paas/v4"
}
```

`apiKey` is required; the factory throws `z.ai provider requires apiKey` without
it. See `apps/server/tests/lib/clients/llm/zai-provider.test.ts` for the
contract.

## Adding a new provider

A provider is "fully wired" when it appears in all of these (use z.ai as the
reference commit):

1. `packages/shared/src/schemas/llm.ts` — add to `LLM_PROVIDERS` and
   `LLMProviderSchema`.
2. `packages/shared/src/constants/urls.ts` — add the API URL to `EXTERNAL_URLS`
   (if it has a canonical endpoint).
3. `apps/server/src/lib/clients/llm/provider.ts` — add a factory and register it
   in `PROVIDER_FACTORIES`.
4. `apps/server/src/agent/provider-factory.ts` — same, agent-side.
5. `apps/agent/lib/llm-providers/` — `types.ts`, `providerTemplates.ts`,
   `providerIcons.tsx`, and `models-dev-data.json`.
