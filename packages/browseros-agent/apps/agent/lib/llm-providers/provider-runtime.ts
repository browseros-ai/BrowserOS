import type { LlmProviderConfig, ProviderType } from './types'

const localRuntimeProviderTypes: ReadonlySet<ProviderType> = new Set([
  'codex',
  'claude-code',
])

/** Identifies provider configs backed by local CLIs instead of HTTP endpoints. */
export function isLocalRuntimeProviderType(type: ProviderType): boolean {
  return localRuntimeProviderTypes.has(type)
}

/**
 * Identifies provider configs that can be sent to the generic chat routes.
 * ACP-backed types (claude-code, codex, acp-custom) are chat-capable: the
 * agent server resolves them to an ACP LanguageModelV2 inside streamText.
 */
export function isChatProviderType(_type: ProviderType): boolean {
  return true
}

/** Finds an exact provider ID only when it is compatible with chat routes. */
export function findChatProviderById(
  providers: LlmProviderConfig[],
  providerId?: string | null,
): LlmProviderConfig | null {
  if (!providerId) return null
  const provider = providers.find((candidate) => candidate.id === providerId)
  return provider && isChatProviderType(provider.type) ? provider : null
}

/**
 * Saved providers are always testable except acp-custom missing a spawn command:
 * acpx's built-in registry resolves claude-code / codex commands, but a custom
 * agent has no fallback so the probe would fail with spawn_failed.
 */
export function canTestProvider(provider: LlmProviderConfig): boolean {
  if (provider.type === 'acp-custom') {
    return Boolean(provider.acpAgentId && provider.acpCommand)
  }
  return true
}

/** Resolves a chat-compatible provider, skipping local runtime configs. */
export function resolveChatProvider(
  providers: LlmProviderConfig[],
  preferredProviderId?: string | null,
): LlmProviderConfig | null {
  const chatProviders = providers.filter((provider) =>
    isChatProviderType(provider.type),
  )
  if (preferredProviderId) {
    const preferred = findChatProviderById(chatProviders, preferredProviderId)
    if (preferred) return preferred
  }
  return chatProviders[0] ?? null
}

/**
 * Scheduled tasks and refine-prompt requests go through the hosted
 * BrowserOS `/chat` endpoint and therefore cannot use local-runtime
 * providers (claude-code, codex, acp-custom) which only exist as a
 * spawned CLI on the user's machine. These helpers explicitly skip
 * those types so the resolver falls back to a cloud-routable provider.
 */
export function findCloudChatProviderById(
  providers: LlmProviderConfig[],
  providerId?: string | null,
): LlmProviderConfig | null {
  if (!providerId) return null
  const provider = providers.find((candidate) => candidate.id === providerId)
  return provider && !isLocalRuntimeProviderType(provider.type)
    ? provider
    : null
}

export function resolveCloudChatProvider(
  providers: LlmProviderConfig[],
  preferredProviderId?: string | null,
): LlmProviderConfig | null {
  const cloudProviders = providers.filter(
    (provider) => !isLocalRuntimeProviderType(provider.type),
  )
  if (preferredProviderId) {
    const preferred = findCloudChatProviderById(
      cloudProviders,
      preferredProviderId,
    )
    if (preferred) return preferred
  }
  return cloudProviders[0] ?? null
}
