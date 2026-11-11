import { expect, test } from 'bun:test'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { start } from './environment.mjs'

test('failed browser startup rejects promptly and removes only its own temporary profiles', async () => {
  const output = mkdtempSync(join(tmpdir(), 'neo-env-test-'))
  try {
    await expect(
      start({
        ports: 19561,
        browser: join(output, 'missing-browser'),
        neoBinary: join(output, 'missing-server'),
        assetsDir: output,
        runDir: output,
        gitSha: 'test',
      }),
    ).rejects.toThrow()
    const children = JSON.parse(readFileSync(join(output, 'processes.json')))
    for (const child of children)
      for (const arg of child.spawnargs)
        if (arg.startsWith('--user-data-dir=')) {
          const profile = arg.slice('--user-data-dir='.length)
          expect(existsSync(profile)).toBe(false)
        }
  } finally {
    rmSync(output, { recursive: true, force: true })
  }
}, 5000)
