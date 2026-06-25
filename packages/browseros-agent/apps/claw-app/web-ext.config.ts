import { createHash } from 'node:crypto'
import { mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineWebExtConfig } from 'wxt'

const env = process.env
const configDir = dirname(fileURLToPath(import.meta.url))

/**
 * Returns a worktree+package-scoped Chromium profile so two dev runs
 * never share state (this extension vs the agent extension; one
 * worktree vs another with the same basename).
 *
 * Label = worktree dir basename (eg. feat-foo-bar)
 * Key   = 8-char sha256 of this package's directory
 * Result: /tmp/browseros-dev-<label>-<key>
 */
function defaultChromiumProfile(): string {
  const worktreeRoot = resolve(configDir, '../../../..')
  const label = sanitizeProfileLabel(basename(worktreeRoot)) || 'repo'
  const key = createHash('sha256').update(configDir).digest('hex').slice(0, 8)
  return join(tmpdir(), `browseros-dev-${label}-${key}`)
}

function sanitizeProfileLabel(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9_.]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

/**
 * Honors an explicit BROWSEROS_USER_DATA_DIR override; otherwise
 * falls back to the worktree-scoped default. Either way the dir is
 * created up-front so Chromium doesn't refuse to start.
 */
function chromiumProfile(): string {
  const configured = env.BROWSEROS_USER_DATA_DIR?.trim()
  const profile = configured || defaultChromiumProfile()
  mkdirSync(profile, { recursive: true })
  return profile
}

const chromiumArgs = [
  '--use-mock-keychain',
  '--show-component-extension-options',
  // The dev BrowserOS binary ships an MCP server on port 9100; this
  // package brings its own (@browseros/claw-server on 9200),
  // so disable the bundled one to avoid port + behaviour drift.
  '--disable-browseros-server',
  '--disable-browseros-extensions',
  '--browseros-dock-icon=dev',
]

if (env.BROWSEROS_CLAW_CDP_PORT) {
  chromiumArgs.push(`--remote-debugging-port=${env.BROWSEROS_CLAW_CDP_PORT}`)
}
if (env.BROWSEROS_SERVER_PORT) {
  chromiumArgs.push(`--browseros-mcp-port=${env.BROWSEROS_SERVER_PORT}`)
  chromiumArgs.push(`--browseros-server-port=${env.BROWSEROS_SERVER_PORT}`)
  // --disable-browseros-server means no proxy is running, so proxy
  // port falls back to server port.
  chromiumArgs.push(`--browseros-proxy-port=${env.BROWSEROS_SERVER_PORT}`)
}
export default defineWebExtConfig({
  binaries: {
    chrome:
      env.BROWSEROS_BINARY ||
      '/Applications/BrowserOS.app/Contents/MacOS/BrowserOS',
  },
  chromiumArgs,
  chromiumProfile: chromiumProfile(),
  keepProfileChanges: true,
  startUrls: ['chrome://newtab'],
})
