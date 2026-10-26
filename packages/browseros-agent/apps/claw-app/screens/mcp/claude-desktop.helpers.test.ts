import { describe, expect, it } from 'bun:test'
import type { ClaudeDesktopExtension } from '@browseros/claw-api'
import { claudeSetupState } from './claude-desktop.helpers'

function extension(
  overrides: Partial<ClaudeDesktopExtension> = {},
): ClaudeDesktopExtension {
  return {
    claudeInstalled: true,
    installed: false,
    availableVersion: '0.5.0',
    updateAvailable: false,
    ...overrides,
  }
}

describe('claudeSetupState', () => {
  it('offers to add neo once Claude is found', () => {
    expect(claudeSetupState(undefined, 'idle')).toBe('checking')
    expect(
      claudeSetupState(extension({ claudeInstalled: false }), 'idle'),
    ).toBe('no-claude')
    expect(claudeSetupState(extension(), 'idle')).toBe('add')
  })

  it('waits for Install in Claude, then shows it connected', () => {
    expect(claudeSetupState(extension(), 'waiting')).toBe('waiting')
    expect(claudeSetupState(extension(), 'stuck')).toBe('stuck')
    expect(
      claudeSetupState(
        extension({ installed: true, installedVersion: '0.5.0' }),
        'waiting',
      ),
    ).toBe('connected')
  })

  it('treats an older install as an update until the new one lands', () => {
    const old = extension({
      installed: true,
      installedVersion: '0.4.0',
      updateAvailable: true,
    })
    expect(claudeSetupState(old, 'idle')).toBe('update')
    expect(claudeSetupState(old, 'waiting')).toBe('waiting')
  })
})
