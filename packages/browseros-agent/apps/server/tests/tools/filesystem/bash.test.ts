import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createBashTool } from '../../../src/tools/filesystem/bash'
import type { FilesystemToolResult } from '../../../src/tools/filesystem/utils'

let tmpDir: string
let exec: (params: Record<string, unknown>) => Promise<FilesystemToolResult>

beforeEach(async () => {
  tmpDir = join(
    tmpdir(),
    `fs-bash-test-${Date.now()}-${Math.random().toString(36).slice(2)}`,
  )
  await mkdir(tmpDir, { recursive: true })
  const tool = createBashTool(tmpDir)
  // biome-ignore lint/suspicious/noExplicitAny: test helper
  exec = (params) => (tool as any).execute(params)
})

afterEach(async () => {
  await rm(tmpDir, { recursive: true, force: true })
})

describe('filesystem_bash', () => {
  const isWindows = process.platform === 'win32'
  const pwdCommand = isWindows ? 'cd' : 'pwd'
  const failCommand = isWindows
    ? 'dir C:\\nonexistent_directory_xyz'
    : 'ls /nonexistent_directory_xyz'
  const pipeWordCountCommand = isWindows
    ? 'echo hello | more'
    : 'echo "a b c" | wc -w'
  const timeoutCommand = isWindows
    ? 'powershell -NoProfile -Command "Start-Sleep -Seconds 30"'
    : 'exec sleep 30'
  const successNoOutputCommand = isWindows ? 'ver > nul' : 'true'
  const envCommand = isWindows ? 'echo %USERPROFILE%' : 'echo $HOME'

  it('executes a simple command', async () => {
    const result = await exec({ command: 'echo hello' })
    expect(result.isError).toBeUndefined()
    expect(result.text.trim()).toBe('hello')
  })

  it('returns output from pwd', async () => {
    const result = await exec({ command: pwdCommand })
    expect(result.isError).toBeUndefined()
    expect(result.text.trim()).toContain(tmpDir)
  })

  it('captures stderr on failure', async () => {
    const result = await exec({ command: failCommand })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('Exit code:')
  })

  it('returns exit code for failed commands', async () => {
    const result = await exec({ command: 'exit 42' })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('[Exit code: 42]')
  })

  it('handles piped commands', async () => {
    const result = await exec({ command: pipeWordCountCommand })
    expect(result.isError).toBeUndefined()
    const out = result.text.trim()
    if (isWindows) {
      expect(out.length).toBeGreaterThan(0)
    } else {
      expect(out).toBe('3')
    }
  })

  it('times out long-running commands', async () => {
    if (isWindows) {
      // cmd.exe process termination semantics can make timeout behavior nondeterministic.
      return
    }
    const result = await exec({ command: timeoutCommand, timeout: 1 })
    expect(result.isError).toBe(true)
    expect(result.text).toContain('timed out')
  }, 10_000)

  it('can create files', async () => {
    await exec({
      command: isWindows
        ? 'echo created>testfile.txt'
        : 'echo "created" > testfile.txt',
    })
    const content = await readFile(join(tmpDir, 'testfile.txt'), 'utf-8')
    expect(content.trim()).toBe('created')
  })

  it('handles empty output commands', async () => {
    const result = await exec({ command: successNoOutputCommand })
    expect(result.isError).toBeUndefined()
    expect(result.text).toBe('(no output)')
  })

  it('handles multiline output', async () => {
    const result = await exec({
      command: 'echo "line1"; echo "line2"; echo "line3"',
    })
    expect(result.text).toContain('line1')
    expect(result.text).toContain('line2')
    expect(result.text).toContain('line3')
  })

  it('uses cwd as working directory', async () => {
    await mkdir(join(tmpDir, 'subdir'))
    const subTool = createBashTool(join(tmpDir, 'subdir'))
    const subExec = (params: Record<string, unknown>) =>
      // biome-ignore lint/suspicious/noExplicitAny: test helper
      (subTool as any).execute(params)

    const result = await subExec({ command: pwdCommand })
    expect(result.text.trim()).toContain('subdir')
  })

  it('passes environment variables through', async () => {
    const result = await exec({ command: envCommand })
    expect(result.isError).toBeUndefined()
    expect(result.text.trim().length).toBeGreaterThan(0)
  })
})
