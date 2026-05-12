import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const source = readFileSync(
  join(import.meta.dir, 'NewAgentDialog.tsx'),
  'utf8',
)

describe('NewAgentDialog defensive custom props', () => {
  it('renders without crash when customCommand is undefined via default prop', () => {
    expect(source).toContain('customCommand?: string')
    expect(source).toContain("customCommand = ''")
  })

  it('renders without crash when all custom props are provided', () => {
    expect(source).toContain('customArgs?: string')
    expect(source).toContain('customLabel?: string')
    expect(source).toContain('customProbeResult?: { healthy: boolean; error?: string } | null')
    expect(source).toContain('customProbeLoading?: boolean')
    expect(source).toContain('onCustomCommandChange?: (command: string) => void')
    expect(source).toContain('onCustomArgsChange?: (args: string) => void')
    expect(source).toContain('onCustomLabelChange?: (label: string) => void')
    expect(source).toContain('onProbeCustom?: () => void')
    expect(source).toContain('onImportAcpx?: () => void')
  })

  it('renders correctly with createRuntime custom and empty command', () => {
    expect(source).toContain("const isCustomRuntime = createRuntime === 'custom'")
    expect(source).toContain("const customBlocked = isCustomRuntime && !customCommand.trim()")
  })

  it('disables create button when createRuntime custom and customCommand is empty', () => {
    expect(source).toContain("createRuntime === 'custom'")
    expect(source).toContain('customCommand.trim().length > 0')
  })

  it('enables create button when createRuntime custom and customCommand has value', () => {
    expect(source).toContain('const canCreate =')
    expect(source).toContain('!customBlocked')
    expect(source).toContain('customCommand.trim().length > 0')
  })
})
