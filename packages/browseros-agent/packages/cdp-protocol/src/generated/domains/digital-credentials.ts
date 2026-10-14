// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { FrameId } from './page'

// ══ Types ══

export type VirtualWalletAction = 'respond' | 'decline' | 'wait' | 'clear'

// ══ Commands ══

export interface SetVirtualWalletBehaviorParams {
  action: VirtualWalletAction
  protocol?: string
  response?: Record<string, unknown>
  frameId?: FrameId
}
