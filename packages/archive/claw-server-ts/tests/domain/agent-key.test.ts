/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { agentKeyFromSlug } from '../../src/domain/agent-key'

describe('AgentKey', () => {
  it('brands a normalized slug without changing its value', () => {
    expect(agentKeyFromSlug('claude-code')).toBe('claude-code')
  })
})
