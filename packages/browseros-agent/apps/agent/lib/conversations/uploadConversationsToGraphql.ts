import type { Conversation } from './conversationStorage'

export async function uploadConversationsToGraphql(
  conversations: Conversation[],
) {
  // Security Hardening: Disabled conversations upload to BrowserOS Cloud
  return
}
