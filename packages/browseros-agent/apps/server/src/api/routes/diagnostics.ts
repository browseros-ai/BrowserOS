/** Public system metadata for the extension's diagnostics snapshot; no environment or identity data. */
import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { platform, release } from 'node:os'
import { promisify } from 'node:util'
import { Hono } from 'hono'

export function parseOsRelease(contents: string): {
  os: string
  osVersion: string | null
} {
  const fields = new Map<string, string>()
  for (const line of contents.split('\n')) {
    const match = /^(NAME|VERSION_ID)=(.*)$/.exec(line)
    if (match)
      fields.set(
        match[1],
        match[2].replace(/^(["'])(.*)\1$/, '$2').slice(0, 200),
      )
  }
  return {
    os: fields.get('NAME') || 'Linux',
    osVersion: fields.get('VERSION_ID') || null,
  }
}

async function systemDetails() {
  if (platform() === 'darwin') {
    // os.release() is the Darwin kernel, not the macOS release shown to users.
    const result = await promisify(execFile)(
      '/usr/bin/sw_vers',
      ['-productVersion'],
      { timeout: 1000 },
    ).catch(() => null)
    return { os: 'macOS', osVersion: result?.stdout.trim() || null }
  }
  if (platform() === 'linux') {
    const contents = await readFile('/etc/os-release', 'utf8').catch(() => '')
    return parseOsRelease(contents)
  }
  return {
    os: platform() === 'win32' ? 'Windows' : platform(),
    osVersion: release(),
  }
}

export function createDiagnosticsRoute(version: string) {
  // OS metadata is stable for this process. Share one read across simultaneous requests.
  let details: ReturnType<typeof systemDetails> | undefined
  return new Hono().get('/', async (c) => {
    details ??= systemDetails()
    return c.json({ version, ...(await details) })
  })
}
