import { describe, expect, test } from 'bun:test'
import { HARNESS_TO_AGENT_ID, harnessEnum } from '../../src/services/harnesses'

describe('live harness definitions', () => {
  test('maps every accepted harness label to its manager agent id', () => {
    expect(harnessEnum.options).toEqual([
      'Claude Code',
      'Codex',
      'Cursor',
      'OpenCode',
      'Antigravity',
      'VS Code',
      'Zed',
    ])
    expect(HARNESS_TO_AGENT_ID).toEqual({
      'Claude Code': 'claude-code',
      Codex: 'codex',
      Cursor: 'cursor',
      OpenCode: 'opencode',
      Antigravity: 'antigravity',
      'VS Code': 'vscode',
      Zed: 'zed',
    })
  })
})
