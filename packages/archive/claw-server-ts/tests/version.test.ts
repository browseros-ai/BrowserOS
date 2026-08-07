import { describe, expect, test } from 'bun:test'
import { resolve } from 'node:path'
import pkg from '../package.json' with { type: 'json' }
import { VERSION } from '../src/version'

describe('Claw server version', () => {
  test('uses the package manifest during direct source execution', () => {
    expect(VERSION).toBe(pkg.version)
  })

  test('prints the package manifest version from the TypeScript entrypoint', async () => {
    const child = Bun.spawn(
      ['bun', resolve(import.meta.dir, '../src/main.ts'), '--version'],
      {
        cwd: resolve(import.meta.dir, '..'),
        stdout: 'pipe',
        stderr: 'pipe',
      },
    )
    const [exitCode, stdout, stderr] = await Promise.all([
      child.exited,
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ])

    expect(exitCode, stderr).toBe(0)
    expect(stdout.trim()).toBe(pkg.version)
  })
})
