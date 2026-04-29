import { storage } from '@wxt-dev/storage'

/**
 * When on, OpenClaw agents in the agent-command and sidepanel chat
 * surfaces route through the unified harness `/agents/:id/chat` SSE and
 * the harness history hook instead of the legacy `/claw/agents/:id/chat`
 * path. Default off — flipped per-user during dogfood, with the
 * intention of flipping the default once stable.
 *
 * Scoping: assumes the OpenClaw agent has a harness record (i.e. it was
 * created via the harness API after dual-creation landed). Legacy
 * gateway-only agents will see empty history under the flag.
 */
export const useAcpxForOpenClawStorage = storage.defineItem<boolean>(
  'local:feature.useAcpxForOpenClaw',
  { fallback: false },
)
