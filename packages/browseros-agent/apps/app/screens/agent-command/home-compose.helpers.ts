export type HomeLlmRoutingMode = 'wait' | 'inline-chat' | 'sidepanel'

export function resolveHomeLlmRoutingMode({
  capabilitiesLoading,
  supportsInlineChat,
}: {
  capabilitiesLoading: boolean
  supportsInlineChat: boolean
}): HomeLlmRoutingMode {
  if (capabilitiesLoading) return 'wait'
  return supportsInlineChat ? 'inline-chat' : 'sidepanel'
}
