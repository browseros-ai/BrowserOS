import type { ClaudeDesktopExtension } from '@browseros/claw-api'

/** Where the person is in adding neo to Claude from this app. */
export type AddPhase = 'idle' | 'waiting' | 'stuck'

export type ClaudeSetupState =
  | 'checking'
  | 'no-claude'
  | 'add'
  | 'update'
  | 'waiting'
  | 'stuck'
  | 'connected'

/** How long to wait for the person to click Install in Claude before offering the guide. */
export const INSTALL_WAIT_MS = 120_000

export function claudeSetupState(
  extension: ClaudeDesktopExtension | undefined,
  phase: AddPhase,
): ClaudeSetupState {
  if (!extension) return 'checking'
  if (extension.installed && !extension.updateAvailable) return 'connected'
  if (phase !== 'idle') return phase
  if (!extension.claudeInstalled) return 'no-claude'
  return extension.installed ? 'update' : 'add'
}
