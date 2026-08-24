import { describe, expect, it } from 'bun:test'
import { splitCommandLine } from '../../../../src/lib/agents/host-acp/parse-command'

describe('splitCommandLine', () => {
  it('splits a plain command and args on whitespace', () => {
    expect(splitCommandLine('npx -y @scope/my-agent-acp --stdio')).toEqual([
      'npx',
      '-y',
      '@scope/my-agent-acp',
      '--stdio',
    ])
  })

  it('collapses repeated whitespace and trims edges', () => {
    expect(splitCommandLine('  bin   a\tb  ')).toEqual(['bin', 'a', 'b'])
  })

  it('keeps quoted segments as single args', () => {
    expect(splitCommandLine('my-agent --path "/opt/some dir" --flag')).toEqual([
      'my-agent',
      '--path',
      '/opt/some dir',
      '--flag',
    ])
  })

  it('treats single quotes as literal', () => {
    expect(splitCommandLine('bin \'--json={"a": 1}\'')).toEqual([
      'bin',
      '--json={"a": 1}',
    ])
  })

  it('keeps Windows backslash paths intact when unquoted', () => {
    expect(splitCommandLine('C:\\Users\\me\\agent.exe --stdio')).toEqual([
      'C:\\Users\\me\\agent.exe',
      '--stdio',
    ])
  })

  it('keeps Windows paths with spaces via double quotes', () => {
    expect(splitCommandLine('"C:\\Program Files\\agent.exe" --stdio')).toEqual([
      'C:\\Program Files\\agent.exe',
      '--stdio',
    ])
  })

  it('throws on an unterminated quote', () => {
    expect(() => splitCommandLine('bin "oops')).toThrow(/unterminated quote/i)
  })
})
