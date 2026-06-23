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
