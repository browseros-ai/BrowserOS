/**
 * Keeps `vendor/playwright/` an unchanged copy of Playwright at the pinned commit: every file
 * listed in VENDOR.md hashes to its listed sha256, and no file is unlisted. Runs in neo-check
 * with no Playwright checkout.
 */

import { expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { readdirSync, readFileSync } from 'node:fs'
import { join, relative } from 'node:path'

const vendorDir = join(import.meta.dir, '../../vendor/playwright')

function listed(): Map<string, string> {
  const manifest = readFileSync(join(vendorDir, 'VENDOR.md'), 'utf8')
  const rows = manifest.matchAll(/^\| `([^`]+)` \| `([0-9a-f]{64})` \|$/gm)
  return new Map([...rows].map(([, path, sha256]) => [path, sha256]))
}

function onDisk(): string[] {
  return readdirSync(vendorDir, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(vendorDir, join(entry.parentPath, entry.name)))
    .filter((path) => path !== 'VENDOR.md')
    .sort()
}

test('every vendored file is listed in VENDOR.md and unchanged', () => {
  const manifest = listed()
  expect(onDisk()).toEqual([...manifest.keys()].sort())
  for (const [path, sha256] of manifest) {
    const bytes = readFileSync(join(vendorDir, path))
    expect({
      path,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    }).toEqual({ path, sha256 })
  }
})
