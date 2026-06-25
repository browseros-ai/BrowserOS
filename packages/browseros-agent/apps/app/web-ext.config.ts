import { createHash } from 'node:crypto'
import { mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineWebExtConfig } from 'wxt'

// biome-ignore lint/style/noProcessEnv: config file needs env access
const env = process.env
const legacySharedProfiles = new Set([
  '/tmp/browseros-dev',
  '/private/tmp/browseros-dev',
])
const configDir = dirname(fileURLToPath(import.meta.url))

/** Returns a worktree-scoped Chromium profile for local BrowserOS dev runs. */
function defaultChromiumProfile(): string {
  const agentRoot = resolve(configDir, '../..')
  const worktreeRoot = resolve(agentRoot, '../..')
  const label = sanitizeProfileLabel(basename(worktreeRoot)) || 'repo'
  const key = createHash('sha256').update(agentRoot).digest('hex').slice(0, 8)
  return join(tmpdir(), `browseros-dev-${label}-${key}`)
}

function sanitizeProfileLabel(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9_.]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

/** Honors explicit profiles but upgrades the old shared temp profile. */
function chromiumProfile(): string {
  const configured = env.BROWSEROS_USER_DATA_DIR?.trim()
  let profile: string
  if (configured && !legacySharedProfiles.has(resolve(configured))) {
    profile = configured
  } else {
    profile = defaultChromiumProfile()
  }
  mkdirSync(profile, { recursive: true })
  return profile
}

const chromiumArgs = [
  '--use-mock-keychain',
  '--show-component-extension-options',
  '--disable-browseros-server',
  '--disable-browseros-extensions',
  '--browseros-dock-icon=dev',
]

if (env.BROWSEROS_CDP_PORT) {
  chromiumArgs.push(`--remote-debugging-port=${env.BROWSEROS_CDP_PORT}`)
}
if (env.BROWSEROS_SERVER_PORT) {
  chromiumArgs.push(`--browseros-mcp-port=${env.BROWSEROS_SERVER_PORT}`)
  chromiumArgs.push(`--browseros-server-port=${env.BROWSEROS_SERVER_PORT}`)
  // --disable-browseros-server means no proxy is running, so proxy port falls back to server port
  chromiumArgs.push(`--browseros-proxy-port=${env.BROWSEROS_SERVER_PORT}`)
}
if (env.BROWSEROS_EXTENSION_PORT) {
  chromiumArgs.push(
    `--browseros-extension-port=${env.BROWSEROS_EXTENSION_PORT}`,
  )
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
