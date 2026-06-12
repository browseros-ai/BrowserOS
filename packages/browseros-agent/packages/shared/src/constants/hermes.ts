/**
 * BrowserOS LLM provider types Hermes can consume. The frontend filters
 * the global provider list to these; the backend `hermes-provider-map`
 * maps them onto Hermes' own provider keys. Keep both sides in sync via
 * this single list — adding a new entry without updating the backend
 * map will cause a 400 at agent-create time.
 *
 * Bedrock is intentionally NOT included yet — it needs multiple env
 * vars (AWS_ACCESS_KEY_ID + secret + region) and a separate UX path.
 */
export const HERMES_SUPPORTED_BROWSEROS_PROVIDER_TYPES = [
  'anthropic',
  'openai',
  'openai-compatible',
  'openrouter',
] as const

export type HermesSupportedBrowserosProviderType =
  (typeof HERMES_SUPPORTED_BROWSEROS_PROVIDER_TYPES)[number]

/**
 * Provider type the `remote-hermes` integration registers under in the
 * shared LLMProvider enum. The chat route uses this to fork into the
 * RemoteHermesService.
 */
export const REMOTE_HERMES_PROVIDER_TYPE = 'remote-hermes' as const

/**
 * `agentKind` sent to the worker on every turn. The worker preserves it
 * for telemetry; it doesn't change dispatch — that's selected by the
 * agent CLI inside the VM (which is always `hermes acp` here).
 */
export const REMOTE_HERMES_AGENT_KIND = 'browseros-remote' as const

/**
 * `agentId` the laptop sends on every turn. v1 runs a single VM-wide
 * agent identity per install; the worker session manager keys sessions
 * by `agentId::threadId` so this constant + the per-conversation
 * threadId is enough to isolate concurrent conversations.
 */
export const REMOTE_HERMES_DEFAULT_AGENT_ID = 'default' as const
