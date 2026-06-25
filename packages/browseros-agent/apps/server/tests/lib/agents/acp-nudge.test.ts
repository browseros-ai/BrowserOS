/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { isNudgeToolName } from '../../../src/lib/agents/acp/nudge'

describe('isNudgeToolName', () => {
  it('matches the bare tool name', () => {
    expect(isNudgeToolName('suggest_app_connection')).toBe(true)
  })

  it('matches the nudge namespace prefix used by acpx-ai-provider', () => {
    // acpx-ai-provider stringifies the runtime tool title as
    // `nudge/suggest_app_connection`; the suffix check has to tolerate
    // it. Mirrors agent-company's isNudgeToolName semantics exactly.
    expect(isNudgeToolName('nudge/suggest_app_connection')).toBe(true)
  })

  it('rejects browser tools and other names', () => {
    expect(isNudgeToolName('browseros/snapshot')).toBe(false)
    expect(isNudgeToolName('snapshot')).toBe(false)
    expect(isNudgeToolName('suggest_schedule')).toBe(false)
    expect(isNudgeToolName('')).toBe(false)
  })
})
