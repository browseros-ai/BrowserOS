import { app, BrowserWindow, dialog, ipcMain, Menu, safeStorage, session, shell, WebContentsView } from 'electron'
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import { homedir } from 'node:os'
import { existsSync } from 'node:fs'
import { appendFile, cp, mkdir, readFile, writeFile } from 'node:fs/promises'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { AgentProgress, AgentResult, AppSurface, BrowserBounds, BrowserState, BrowserTabState, DesktopPreferences } from './shared/desktop-api.js'

const __dirname = dirname(fileURLToPath(import.meta.url))
const DEFAULT_SERVER_PORT = 9105
const CDP_PORT = 9225
const DEFAULT_URL = 'https://www.google.com'
const DEFAULT_AGENT_PANEL_WIDTH = 420
const MIN_AGENT_PANEL_WIDTH = 340
const MAX_AGENT_PANEL_WIDTH = 600
type AssistantMode = 'docked' | 'floating'

let mainWindow: BrowserWindow | null = null
type BrowserTab = {
  id: number
  windowId: number
  profileId: string
  view: WebContentsView
  state: BrowserState
  groupId?: number
}
type BrowserTabGroup = {
  id: number
  title: string
  color: string
  collapsed: boolean
  windowId: number
}

type DownloadController = {
  cancel: () => void
  pause: () => void
  resume: () => void
  removeFile: () => void
}

let nextTabId = 1
let nextTabGroupId = 1
let nextWindowId = 2
let activeTabId = 0
let activeWindowId = 1
const browserTabs = new Map<number, BrowserTab>()
const browserTabGroups = new Map<number, BrowserTabGroup>()
const desktopWindows = new Map<number, BrowserWindow>()
const activeTabByWindow = new Map<number, number>()
type BrowserProfile = { id: string; name: string; createdAt: number }
const browserProfiles: BrowserProfile[] = [{ id: 'default', name: 'Personal', createdAt: Date.now() }]
let activeProfileId = 'default'
let firstRun = false
type SavedBrowserTab = { profileId: string; url: string; active: boolean; groupId?: number }
let savedBrowserTabs: SavedBrowserTab[] = []
let contentBounds: BrowserBounds = { x: 256, y: 96, width: 832, height: 796 }
let agentView: WebContentsView | null = null
let appView: WebContentsView | null = null
let activeSurface: AppSurface = 'browser'
let agentUiServer: ReturnType<typeof createServer> | null = null
let agentUiBaseUrl = ''
let agentUiServerBaseUrl = ''
let agentUiExtensionBaseUrl = ''
let agentExtensionRoot = ''
const profileExtensionLoads = new Map<string, Promise<void>>()
let assistantVisible = true
let assistantWidth = DEFAULT_AGENT_PANEL_WIDTH
let assistantMode: AssistantMode = 'docked'
let serverProcess: ChildProcessWithoutNullStreams | null = null
let serverReady = false
let serverPort = DEFAULT_SERVER_PORT
let serverUrl = `http://127.0.0.1:${serverPort}`
let latestBrowserState: BrowserState = {
  id: 0,
  url: DEFAULT_URL,
  title: 'Google',
  canGoBack: false,
  canGoForward: false,
  loading: false,
}

// Electron's remote debugging endpoint lets the existing BrowserOS server use
// its browser-core/MCP stack. The server runs in Electron mode and translates
// BrowserOS Browser.* calls to standard Target.* CDP calls.
app.commandLine.appendSwitch('remote-debugging-port', String(CDP_PORT))
app.commandLine.appendSwitch('remote-allow-origins', '*')
const hasSingleInstanceLock = app.requestSingleInstanceLock()
if (!hasSingleInstanceLock) app.quit()
app.on('second-instance', () => {
  if (!mainWindow || mainWindow.isDestroyed()) {
    void createWindow().catch((error) => emitLog(`Could not restore the Request Browser window: ${String(error)}`))
    return
  }
  if (mainWindow.isMinimized()) mainWindow.restore()
  if (!mainWindow.isVisible()) mainWindow.show()
  mainWindow.focus()
})

function getAgentRoot(): string {
  if (app.isPackaged) return process.resourcesPath
  return process.env.REQUEST_BROWSER_AGENT_ROOT
    ? resolve(process.env.REQUEST_BROWSER_AGENT_ROOT)
    : resolve(__dirname, '../../..')
}

function getRuntimeDir(): string {
  const base = process.platform === 'win32'
    ? join(process.env.PUBLIC ?? 'C:\\Users\\Public', 'RequestBrowser')
    : app.getPath('userData')
  return join(base, 'runtime')
}

function getAgentUiRoot(): string {
  if (app.isPackaged) return join(process.resourcesPath, 'browseros-ui')
  const developmentBuild = resolve(getAgentRoot(), 'apps/app/dist/chrome-mv3-dev')
  return existsSync(developmentBuild)
    ? developmentBuild
    : resolve(getAgentRoot(), 'apps/app/dist/chrome-mv3')
}

function contentTypeFor(pathname: string): string {
  const extension = pathname.split('.').pop()?.toLowerCase()
  return {
    html: 'text/html; charset=utf-8',
    js: 'text/javascript; charset=utf-8',
    css: 'text/css; charset=utf-8',
    json: 'application/json; charset=utf-8',
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    svg: 'image/svg+xml',
    ico: 'image/x-icon',
    woff: 'font/woff',
    woff2: 'font/woff2',
    onnx: 'application/octet-stream',
    wasm: 'application/wasm',
  }[extension ?? ''] ?? 'application/octet-stream'
}

async function startAgentUiServer(): Promise<void> {
  const root = getAgentUiRoot()
  if (!existsSync(join(root, 'sidepanel.html'))) {
    emitLog(`Request Browser UI build not found at ${root}`)
    return
  }

  agentUiServer = createServer(async (request: IncomingMessage, response: ServerResponse) => {
    const rawPath = new URL(request.url ?? '/', 'http://127.0.0.1').pathname
    const relativePath = rawPath === '/' ? 'sidepanel.html' : rawPath.replace(/^\/+/, '')
    const candidate = resolve(root, relativePath)
    if (!candidate.startsWith(resolve(root))) {
      response.writeHead(403)
      response.end('Forbidden')
      return
    }
    try {
      let body = await readFile(candidate)
      if (candidate.endsWith('.html')) {
        let html = body.toString('utf8')
        let deferredModule = ''
        if (candidate.endsWith('app.html')) {
          html = html.replace(/<script type="module"[^>]*src="([^"]+)"[^>]*><\/script>/, (_match, source: string) => {
            deferredModule = source
            return ''
          })
        }
        body = Buffer.from(html.replace('</body>', `<script>
          (async () => {
            for (let attempt = 0; attempt < 100 && (!window.chrome || !window.chrome.runtime); attempt += 1) {
              await new Promise(resolve => setTimeout(resolve, 0));
            }
          try {
            if (window.__requestBrowserChrome) {
              const bridge = window.__requestBrowserChrome;
              const scripting = bridge.scripting;
              const chromeBridge = {
                ...bridge,
                scripting: scripting && scripting.executeScriptSerialized
                  ? {
                      ...scripting,
                      executeScript: (details) => scripting.executeScriptSerialized({
                        ...details,
                        ...(typeof details?.func === 'function' ? { func: details.func.toString() } : {}),
                      }),
                    }
                  : scripting,
              };
              Object.defineProperty(window, 'chrome', { value: chromeBridge, configurable: true });
              window.browser = chromeBridge;
            }
            if (window.chrome && window.chrome.runtime && !window.chrome.runtime.id) {
              Object.defineProperty(window.chrome.runtime, 'id', { value: 'request-browser-electron', configurable: true });
            }
          } catch {}
          window.browser = window.chrome;
          ${deferredModule ? `await import(${JSON.stringify(deferredModule)});` : ''}
          })();
        </script></body>`))
      }
      response.writeHead(200, {
        'content-type': contentTypeFor(candidate),
        'cache-control': 'no-store',
        'access-control-allow-origin': '*',
      })
      response.end(body)
    } catch {
      response.writeHead(404)
      response.end('Not found')
    }
  })

  await new Promise<void>((resolvePromise, reject) => {
    agentUiServer?.once('error', reject)
    agentUiServer?.listen(0, '127.0.0.1', () => resolvePromise())
  })
  const address = agentUiServer.address()
  if (!address || typeof address === 'string') throw new Error('Could not start Request Browser UI server')
  agentUiBaseUrl = `http://127.0.0.1:${address.port}`
  agentUiServerBaseUrl = agentUiBaseUrl
  emitLog(`Request Browser UI loaded from ${root}`)
}

async function loadAgentExtension(): Promise<void> {
  const sourceRoot = getAgentUiRoot()
  const root = join(getRuntimeDir(), 'browseros-ui-extension')
  try {
    await mkdir(root, { recursive: true })
    await cp(sourceRoot, root, { recursive: true, force: true })
    agentExtensionRoot = root
    let sourceManifest: Record<string, unknown> = {}
    try {
      sourceManifest = JSON.parse(await readFile(join(sourceRoot, 'manifest.json'), 'utf8')) as Record<string, unknown>
    } catch {
      emitLog('Request Browser manifest was not found; using the desktop fallback manifest.')
    }

    // Keep the legacy generated declarative surface (options, action,
    // side-panel, new-tab route, permissions and content scripts). The
    // original service worker is intentionally omitted: it assumes Chrome's
    // native BrowserOS APIs and would crash inside Electron before the
    // desktop IPC bridge can provide them.
    const {
      background: _background,
      key: _key,
      update_url: _updateUrl,
      externally_connectable: _externallyConnectable,
      web_accessible_resources: _webAccessibleResources,
      content_scripts: sourceContentScripts,
      ...supportedManifest
    } = sourceManifest
    const contentScripts = Array.isArray(sourceContentScripts)
      ? sourceContentScripts.filter((entry) => {
        if (!entry || typeof entry !== 'object') return false
        const scripts = (entry as { js?: unknown }).js
        return !(Array.isArray(scripts) && scripts.some((script) => script === 'content-scripts/auth.js'))
      })
      : undefined
    await writeFile(join(root, 'request-browser-background.js'), `
      chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
        if (!message || typeof message.type !== 'string') return false
        if (message.type === 'runtime.getTabId') {
          sendResponse({ res: { tabId: sender.tab && sender.tab.id } })
          return true
        }
        // These messages are owned by the Electron main-process bridge when
        // they originate from the app/side panel. Content-script callers get
        // a successful no-op response instead of an unhandled Promise error.
        if (
          message.type === 'runtime.authSuccess' ||
          message.type === 'runtime.stopAgent' ||
          message.type === 'runtime.sidePanelScopeChanged'
        ) {
          sendResponse({ res: undefined })
          return true
        }
        return false
      })
    `)
    await writeFile(join(root, 'manifest.json'), `${JSON.stringify({
      ...supportedManifest,
      manifest_version: 3,
      name: 'Request Browser UI',
      version: '0.1.0',
      description: 'Request Browser AI interface',
      background: { service_worker: 'request-browser-background.js' },
      ...(contentScripts?.length ? { content_scripts: contentScripts } : {}),
      content_security_policy: {
        extension_pages: "script-src 'self'; object-src 'self'",
      },
    }, null, 2)}\n`)
    const extension = await session.defaultSession.extensions.loadExtension(root, {
      allowFileAccess: true,
    })
    agentUiExtensionBaseUrl = `chrome-extension://${extension.id}`
    // Keep the extension registered for its generated manifest/content
    // scripts, but render the app through the local server. Electron skips
    // the preload bridge on chrome-extension:// pages, which would leave the
    // BrowserOS UI connected to Chrome's incomplete native runtime object.
    agentUiBaseUrl = agentUiServerBaseUrl || agentUiExtensionBaseUrl
    emitLog(`Request Browser extension loaded as ${extension.id}; UI bridge served from ${agentUiBaseUrl}`)
  } catch (error) {
    emitLog(`Could not load Request Browser as an Electron extension; using local UI server: ${String(error)}`)
  }
}

async function ensureProfileExtension(profileId: string): Promise<void> {
  if (!agentExtensionRoot) return
  const existing = profileExtensionLoads.get(profileId)
  if (existing) return existing

  const profileSession = session.fromPartition(`persist:request-browser-${profileId}`)
  const load = profileSession.extensions.loadExtension(agentExtensionRoot, { allowFileAccess: true })
    .then(() => undefined)
    .catch((error) => {
      profileExtensionLoads.delete(profileId)
      emitLog(`Could not load Request Browser content scripts for profile ${profileId}: ${String(error)}`)
    })
  profileExtensionLoads.set(profileId, load)
  return load
}

function getBrowserState(): BrowserState {
  const tab = browserTabs.get(activeTabId)
  if (!tab) return latestBrowserState
  return {
    ...tab.state,
    canGoBack: tab.view.webContents.canGoBack(),
    canGoForward: tab.view.webContents.canGoForward(),
  }
}

function listBrowserTabs(): BrowserTabState[] {
  return [...browserTabs.values()].filter((tab) => tab.profileId === activeProfileId && tab.windowId === 1).map((tab) => ({
    ...tab.state,
    canGoBack: tab.view.webContents.canGoBack(),
    canGoForward: tab.view.webContents.canGoForward(),
    active: activeTabByWindow.get(tab.windowId) === tab.id,
  }))
}

function broadcastBrowserState(): void {
  latestBrowserState = getBrowserState()
  mainWindow?.webContents.send('browser:state', latestBrowserState)
  mainWindow?.webContents.send('browser:tabs', listBrowserTabs())
  agentView?.webContents.send('agent:tabs-updated', latestBrowserState.id, { status: latestBrowserState.loading ? 'loading' : 'complete', url: latestBrowserState.url, title: latestBrowserState.title }, currentTab())
}

function normalizeNavigationInput(input: string): string {
  const value = input.trim()
  if (!value) return DEFAULT_URL
  try {
    const parsed = new URL(value)
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') return parsed.toString()
  } catch {
    // Treat non-URLs as search terms below.
  }
  return `https://www.google.com/search?q=${encodeURIComponent(value)}`
}

function attachShortcuts(contents: Electron.WebContents): void {
  contents.on('before-input-event', (event, input) => {
    if (input.type === 'keyDown' && input.key === 'F11') {
      event.preventDefault()
      mainWindow?.setFullScreen(!mainWindow.isFullScreen())
      return
    }
    if (!(input.control || input.meta) || input.type !== 'keyDown') return
    const key = input.key.toLowerCase()
    if (key === 't') { event.preventDefault(); void createBrowserTab(DEFAULT_URL, true) }
    else if (key === 'w') { event.preventDefault(); void closeBrowserTab(activeTabId) }
    else if (key === 'l') { event.preventDefault(); mainWindow?.webContents.send('app:focus-address') }
    else if (key === 'b' && !input.shift) { event.preventDefault(); mainWindow?.webContents.send('app:toggle-sidebar') }
    else if (key === 'h') { event.preventDefault(); void openSurface('history') }
    else if (key === 'j') { event.preventDefault(); void openSurface('downloads') }
    else if (key === 'p') { event.preventDefault(); browserTabs.get(activeTabId)?.view.webContents.print() }
    else if (key === '0') { event.preventDefault(); browserTabs.get(activeTabId)?.view.webContents.setZoomFactor(1) }
    else if (key === '+' || key === '=') { event.preventDefault(); const target = browserTabs.get(activeTabId)?.view.webContents; if (target) target.setZoomFactor(Math.min(3, target.getZoomFactor() + 0.1)) }
    else if (key === '-') { event.preventDefault(); const target = browserTabs.get(activeTabId)?.view.webContents; if (target) target.setZoomFactor(Math.max(0.25, target.getZoomFactor() - 0.1)) }
  })
}

async function navigate(input: string): Promise<BrowserState> {
  const tab = browserTabs.get(activeTabId)
  if (!tab) return getBrowserState()
  const url = normalizeNavigationInput(input)
  // Do not make the omnibox wait for a full page-load promise. Modern sites
  // intentionally keep requests open, while the visible document is already
  // usable. Loading state and the final URL are broadcast by the WebContents
  // navigation listeners below.
  tab.state = {
    ...tab.state,
    url,
    loading: true,
  }
  broadcastBrowserState()
  void tab.view.webContents.loadURL(url).catch((error) => {
    tab.state = { ...tab.state, loading: false }
    broadcastBrowserState()
    emitLog(`Navigation failed for tab ${tab.id} (${String(error)})`)
  })
  return { ...tab.state }
}

async function importBookmarksFromHtml(): Promise<void> {
  if (!mainWindow) return
  const selection = await dialog.showOpenDialog(mainWindow, {
    title: 'Import bookmarks from Chrome or another browser',
    filters: [{ name: 'Bookmark HTML', extensions: ['html', 'htm'] }],
    properties: ['openFile'],
  })
  if (selection.canceled || !selection.filePaths[0]) return
  const html = await readFile(selection.filePaths[0], 'utf8')
  const matches = [...html.matchAll(/<a\b[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi)]
  let imported = 0
  for (const match of matches) {
    const url = match[1]
    if (!/^https?:/i.test(url) || bookmarks.some((bookmark) => bookmark.url === url)) continue
    const title = match[2].replace(/<[^>]+>/g, '').replaceAll('&amp;', '&').replaceAll('&quot;', '"').trim() || url
    bookmarks.push({ id: crypto.randomUUID(), title, url, dateAdded: Date.now() })
    imported += 1
  }
  scheduleDesktopStateSave()
  await dialog.showMessageBox(mainWindow, { type: 'info', title: 'Import complete', message: `Imported ${imported} bookmarks.`, buttons: ['Done'] })
}

function emitLog(line: string): void {
  mainWindow?.webContents.send('app:log', line)
  void appendFile(join(getRuntimeDir(), 'agent.log'), `${new Date().toISOString()} ${line}\n`).catch(() => {})
}

function stopAgentServer(): void {
  const pid = serverProcess?.pid
  if (!pid) return
  serverProcess?.kill()
  if (process.platform === 'win32') {
    spawn('taskkill.exe', ['/PID', String(pid), '/T', '/F'], {
      stdio: 'ignore',
      windowsHide: true,
    })
  }
  serverProcess = null
  serverReady = false
}

function getOpenCodeAcpCommand(): string {
  return `${join(getRuntimeDir(), 'opencode.cmd').replaceAll('\\', '/')} acp`
}

async function ensureOpenCodeLauncher(): Promise<void> {
  if (process.platform !== 'win32' || !process.env.APPDATA) return
  const installedCommand = join(process.env.APPDATA, 'npm', 'opencode.cmd')
  if (!existsSync(installedCommand)) return
  const launcher = join(getRuntimeDir(), 'opencode.cmd')
  await mkdir(dirname(launcher), { recursive: true })
  await writeFile(
    launcher,
    `@echo off\r\ncall "${installedCommand}" %*\r\n`,
  )
}

const extensionStorage = new Map<string, unknown>([
  ['default-provider-id', 'opencode-zen'],
  [
    'llm-providers',
    [
      {
        id: 'opencode-zen',
        type: 'opencode-zen',
        name: 'OpenCode Zen (Free models)',
        modelId: 'opencode/mimo-v2.5-free',
        supportsImages: true,
        contextWindow: 200000,
        temperature: 0.2,
        acpAgentId: 'opencode',
        acpCommand: getOpenCodeAcpCommand(),
        createdAt: Date.now(),
        updatedAt: Date.now(),
      },
      {
        id: 'opencode-go',
        type: 'opencode-go',
        name: 'OpenCode Go',
        modelId: 'opencode-go/mimo-v2.5',
        supportsImages: true,
        contextWindow: 200000,
        temperature: 0.2,
        acpAgentId: 'opencode',
        acpCommand: getOpenCodeAcpCommand(),
        createdAt: Date.now(),
        updatedAt: Date.now(),
      },
    ],
  ],
])

function migrateOpenCodeProviders(): void {
  const value = extensionStorage.get('llm-providers')
  if (!Array.isArray(value)) return
  const providers = value.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
  let migratedLegacyId: string | undefined
  for (const provider of providers) {
    if (provider.type !== 'opencode') continue
    migratedLegacyId = typeof provider.id === 'string' ? provider.id : undefined
    provider.type = 'opencode-go'
    provider.name = 'OpenCode Go'
    if (typeof provider.modelId !== 'string' || !provider.modelId.startsWith('opencode-go/')) {
      provider.modelId = 'opencode-go/mimo-v2.5'
    }
    provider.acpCommand = getOpenCodeAcpCommand()
  }
  for (const provider of providers) {
    if (provider.type === 'opencode-go' || provider.type === 'opencode-zen') {
      provider.acpAgentId = 'opencode'
      provider.acpCommand = getOpenCodeAcpCommand()
    }
  }
  if (!providers.some((provider) => provider.type === 'opencode-zen')) {
    providers.unshift({
      id: 'opencode-zen', type: 'opencode-zen', name: 'OpenCode Zen (Free models)',
      modelId: 'opencode/mimo-v2.5-free', supportsImages: true,
      contextWindow: 200000, temperature: 0.2, acpAgentId: 'opencode',
      acpCommand: getOpenCodeAcpCommand(), createdAt: Date.now(), updatedAt: Date.now(),
    })
  }
  extensionStorage.set('llm-providers', providers)
  if (extensionStorage.get('default-provider-id') === migratedLegacyId) {
    extensionStorage.set('default-provider-id', 'opencode-zen')
  }
}

async function syncOpenCodeCredentials(): Promise<void> {
  const providers = extensionStorage.get('llm-providers')
  if (!Array.isArray(providers)) return
  const authPath = join(homedir(), '.local', 'share', 'opencode', 'auth.json')
  let auth: Record<string, unknown> = {}
  try {
    auth = JSON.parse(await readFile(authPath, 'utf8')) as Record<string, unknown>
  } catch {}
  let changed = false
  for (const item of providers) {
    if (!item || typeof item !== 'object') continue
    const provider = item as Record<string, unknown>
    if (typeof provider.apiKey !== 'string' || !provider.apiKey) continue
    const authId = provider.type === 'opencode-zen' ? 'opencode' :
      provider.type === 'opencode-go' || provider.type === 'opencode' ? 'opencode-go' : undefined
    if (!authId) continue
    auth[authId] = { type: 'api', key: provider.apiKey }
    changed = true
  }
  if (!changed) return
  await mkdir(dirname(authPath), { recursive: true })
  await writeFile(authPath, `${JSON.stringify(auth, null, 2)}\n`, { mode: 0o600 })
}
const browserPrefs = new Map<string, unknown>([
  ['browseros.server.version', '0.0.126'],
  ['browseros.server.mcp_port', DEFAULT_SERVER_PORT],
  ['browseros.server.proxy_port', DEFAULT_SERVER_PORT],
  ['browseros.server.server_port', DEFAULT_SERVER_PORT],
  ['browseros.third_party_llm.providers', '[]'],
  ['browseros.providers', '[]'],
  ['request-browser.ad-blocking.enabled', true],
])
function getDesktopPreferences(): DesktopPreferences {
  return {
    showLlmChat: browserPrefs.get('browseros.show_llm_chat') !== false,
    showToolbarLabels: browserPrefs.get('browseros.show_toolbar_labels') !== false,
  }
}

function broadcastDesktopPreferences(): void {
  mainWindow?.webContents.send('app:preferences', getDesktopPreferences())
}
const TRACKER_HOSTS = [
  'doubleclick.net',
  'googlesyndication.com',
  'google-analytics.com',
  'adservice.google.com',
  'connect.facebook.net',
  'analytics.twitter.com',
]
type BookmarkEntry = { id: string; title: string; url: string; dateAdded: number }
type HistoryEntry = { id: string; title: string; url: string; lastVisitTime: number; visitCount: number }
type DownloadEntry = { id: string; url: string; filename: string; path?: string; startTime: string; state: 'in_progress' | 'complete' | 'interrupted'; bytesReceived: number; totalBytes: number }
const bookmarks: BookmarkEntry[] = []
const historyEntries: HistoryEntry[] = []
const downloadEntries: DownloadEntry[] = []
const grantedPermissions = new Set<string>()
type AlarmEntry = { name: string; scheduledTime: number; periodInMinutes?: number }
const alarms = new Map<string, AlarmEntry>()
const alarmTimers = new Map<string, NodeJS.Timeout>()
type ScheduledJob = { id: string; name: string; query: string; enabled: boolean; providerId?: string; lastRunAt?: string }
type ScheduledJobRun = { id: string; jobId: string; startedAt: string; completedAt?: string; status: 'running' | 'completed' | 'failed'; result?: string; error?: string }
const scheduledRunControllers = new Map<string, AbortController>()
const downloadControllers = new Map<string, DownloadController>()
const injectedStyles = new Map<number, Map<string, string>>()

const SENSITIVE_KEY = /(api.?key|secret|token|password|credential)/i
const ENCRYPTED_VALUE = '__request_browser_encrypted__'

function stateFilePath(): string {
  return join(app.getPath('userData'), 'desktop-state.json')
}

function protectSecrets(value: unknown, key = ''): unknown {
  if (typeof value === 'string' && value && SENSITIVE_KEY.test(key) && safeStorage.isEncryptionAvailable()) {
    return { [ENCRYPTED_VALUE]: safeStorage.encryptString(value).toString('base64') }
  }
  if (Array.isArray(value)) return value.map((item) => protectSecrets(item, key))
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([childKey, child]) => [childKey, protectSecrets(child, childKey)]))
  }
  return value
}

function revealSecrets(value: unknown): unknown {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const record = value as Record<string, unknown>
    if (typeof record[ENCRYPTED_VALUE] === 'string' && safeStorage.isEncryptionAvailable()) {
      try {
        return safeStorage.decryptString(Buffer.from(record[ENCRYPTED_VALUE], 'base64'))
      } catch {
        return ''
      }
    }
    return Object.fromEntries(Object.entries(record).map(([key, child]) => [key, revealSecrets(child)]))
  }
  if (Array.isArray(value)) return value.map(revealSecrets)
  return value
}

async function loadDesktopState(): Promise<void> {
  firstRun = !existsSync(stateFilePath())
  try {
    const state = JSON.parse(await readFile(stateFilePath(), 'utf8')) as {
      extensionStorage?: Record<string, unknown>
      browserPrefs?: Record<string, unknown>
      bookmarks?: BookmarkEntry[]
      history?: HistoryEntry[]
      downloads?: DownloadEntry[]
      permissions?: string[]
      profiles?: BrowserProfile[]
      activeProfileId?: string
      tabs?: SavedBrowserTab[]
      tabGroups?: BrowserTabGroup[]
      alarms?: AlarmEntry[]
    }
    for (const [key, value] of Object.entries(state.extensionStorage ?? {})) extensionStorage.set(key, revealSecrets(value))
    for (const [key, value] of Object.entries(state.browserPrefs ?? {})) browserPrefs.set(key, revealSecrets(value))
    const storedServerPort = browserPrefs.get('browseros.server.server_port')
    if (typeof storedServerPort === 'number' && storedServerPort >= 1024 && storedServerPort <= 65535) {
      serverPort = storedServerPort
      serverUrl = `http://127.0.0.1:${serverPort}`
    }
    // The Electron shell exposes one local HTTP/MCP server, so all three
    // BrowserOS port preferences must point to the same listener.
    browserPrefs.set('browseros.server.server_port', serverPort)
    browserPrefs.set('browseros.server.mcp_port', serverPort)
    browserPrefs.set('browseros.server.proxy_port', serverPort)
    assistantVisible = browserPrefs.get('browseros.side_panel.enabled') !== false && browserPrefs.get('browseros.show_llm_chat') !== false
    const storedAssistantWidth = browserPrefs.get('request-browser.ai-panel.width')
    if (typeof storedAssistantWidth === 'number') {
      assistantWidth = Math.min(MAX_AGENT_PANEL_WIDTH, Math.max(MIN_AGENT_PANEL_WIDTH, storedAssistantWidth))
    }
    const storedAssistantMode = browserPrefs.get('request-browser.ai-panel.mode')
    if (storedAssistantMode === 'docked' || storedAssistantMode === 'floating') {
      assistantMode = storedAssistantMode
    }
    bookmarks.splice(0, bookmarks.length, ...(state.bookmarks ?? []))
    historyEntries.splice(0, historyEntries.length, ...(state.history ?? []))
    downloadEntries.splice(0, downloadEntries.length, ...(state.downloads ?? []))
    for (const permission of state.permissions ?? []) grantedPermissions.add(permission)
    if (state.profiles?.length) browserProfiles.splice(0, browserProfiles.length, ...state.profiles)
    if (state.activeProfileId && browserProfiles.some((profile) => profile.id === state.activeProfileId)) activeProfileId = state.activeProfileId
    savedBrowserTabs = state.tabs ?? []
    browserTabGroups.clear()
    for (const group of state.tabGroups ?? []) {
      browserTabGroups.set(group.id, group)
      nextTabGroupId = Math.max(nextTabGroupId, group.id + 1)
    }
    for (const alarm of state.alarms ?? []) alarms.set(alarm.name, alarm)
  } catch {
    // The defaults above are the first-run state.
  }
}

let saveStateTimer: NodeJS.Timeout | undefined
function scheduleDesktopStateSave(): void {
  if (saveStateTimer) clearTimeout(saveStateTimer)
  saveStateTimer = setTimeout(() => {
    const payload = {
      extensionStorage: protectSecrets(Object.fromEntries(extensionStorage)),
      browserPrefs: protectSecrets(Object.fromEntries(browserPrefs)),
      bookmarks,
      history: historyEntries.slice(0, 5000),
      downloads: downloadEntries.slice(0, 1000),
      permissions: [...grantedPermissions],
      profiles: browserProfiles,
      activeProfileId,
      tabs: [...browserTabs.values()].filter((tab) => tab.windowId === 1 && /^https?:|^about:/i.test(tab.state.url)).map((tab) => ({ profileId: tab.profileId, url: tab.state.url, active: activeTabByWindow.get(1) === tab.id, ...(tab.groupId ? { groupId: tab.groupId } : {}) })),
      tabGroups: [...browserTabGroups.values()],
      alarms: [...alarms.values()],
    }
    void mkdir(dirname(stateFilePath()), { recursive: true })
      .then(() => writeFile(stateFilePath(), `${JSON.stringify(payload, null, 2)}\n`))
      .catch((error) => emitLog(`Could not persist desktop settings: ${String(error)}`))
  }, 100)
}

function scheduleAlarm(alarm: AlarmEntry): void {
  const existing = alarmTimers.get(alarm.name)
  if (existing) clearTimeout(existing)
  const delay = Math.max(0, alarm.scheduledTime - Date.now())
  const timer = setTimeout(() => {
    agentView?.webContents.send('agent:alarm', alarm)
    appView?.webContents.send('agent:alarm', alarm)
    if (alarm.name.startsWith('scheduled-job-')) {
      void runScheduledJob(alarm.name.replace('scheduled-job-', ''))
    }
    if (alarm.periodInMinutes) {
      alarm.scheduledTime = Date.now() + alarm.periodInMinutes * 60_000
      scheduleAlarm(alarm)
    } else {
      alarms.delete(alarm.name)
      alarmTimers.delete(alarm.name)
    }
    scheduleDesktopStateSave()
  }, Math.min(delay, 2_147_483_647))
  alarmTimers.set(alarm.name, timer)
}

function restoreAlarms(): void {
  for (const alarm of alarms.values()) scheduleAlarm(alarm)
}

function recordHistory(url: string, title: string): void {
  if (!/^https?:/i.test(url)) return
  const existing = historyEntries.find((entry) => entry.url === url)
  if (existing) {
    existing.title = title || existing.title
    existing.lastVisitTime = Date.now()
    existing.visitCount += 1
  } else {
    historyEntries.unshift({ id: crypto.randomUUID(), url, title: title || url, lastVisitTime: Date.now(), visitCount: 1 })
  }
  scheduleDesktopStateSave()
}

const downloadTrackingProfiles = new Set<string>()
const permissionTrackingProfiles = new Set<string>()
function ensureProfileSession(profileId: string): void {
  const profileSession = session.fromPartition(`persist:request-browser-${profileId}`)
  if (!permissionTrackingProfiles.has(profileId)) {
    permissionTrackingProfiles.add(profileId)
    profileSession.setPermissionCheckHandler((webContents, permission, requestingOrigin) => {
      const origin = requestingOrigin || webContents?.getURL() || 'unknown-origin'
      return grantedPermissions.has(`${profileId}:${origin}:${permission}`)
    })
    profileSession.setPermissionRequestHandler((webContents, permission, callback, details) => {
      let origin = details.requestingUrl || webContents?.getURL() || 'unknown-origin'
      try { origin = new URL(origin).origin } catch { /* Keep the requesting URL as the label. */ }
      const permissionKey = `${profileId}:${origin}:${permission}`
      if (grantedPermissions.has(permissionKey)) {
        callback(true)
        return
      }
      if (!mainWindow) {
        callback(false)
        return
      }
      void dialog.showMessageBox(mainWindow, {
        type: 'question',
        title: 'Site permission',
        message: `${origin} wants permission to use ${permission}.`,
        detail: 'Allow this permission for the current browser profile?',
        buttons: ['Block', 'Allow'],
        defaultId: 0,
        cancelId: 0,
        noLink: true,
      }).then(({ response }) => {
        const allowed = response === 1
        if (allowed) {
          grantedPermissions.add(permissionKey)
          scheduleDesktopStateSave()
        }
        callback(allowed)
      }).catch(() => callback(false))
    })
    profileSession.webRequest.onBeforeRequest({ urls: ['*://*/*'] }, (details, callback) => {
      if (browserPrefs.get('request-browser.ad-blocking.enabled') !== true) {
        callback({ cancel: false })
        return
      }
      let hostname = ''
      try { hostname = new URL(details.url).hostname } catch { /* Ignore invalid request URLs. */ }
      callback({ cancel: TRACKER_HOSTS.some((blocked) => hostname === blocked || hostname.endsWith(`.${blocked}`)) })
    })
  }
  if (downloadTrackingProfiles.has(profileId)) return
  downloadTrackingProfiles.add(profileId)
  profileSession.on('will-download', (_event, item) => {
    const entry: DownloadEntry = {
      id: crypto.randomUUID(),
      url: item.getURL(),
      filename: item.getFilename(),
      startTime: new Date().toISOString(),
      state: 'in_progress',
      bytesReceived: 0,
      totalBytes: item.getTotalBytes(),
    }
    downloadControllers.set(entry.id, {
      cancel: () => item.cancel(),
      pause: () => item.pause(),
      resume: () => item.resume(),
      removeFile: () => {
        const path = item.getSavePath()
        if (path) void shell.trashItem(path)
      },
    })
    downloadEntries.unshift(entry)
    scheduleDesktopStateSave()
    item.on('updated', (_downloadEvent, state) => {
      entry.bytesReceived = item.getReceivedBytes()
      entry.totalBytes = item.getTotalBytes()
      if (state === 'interrupted') entry.state = 'interrupted'
      scheduleDesktopStateSave()
      agentView?.webContents.send('agent:download-changed', entry)
    })
    item.once('done', (_downloadEvent, state) => {
      entry.path = item.getSavePath()
      entry.bytesReceived = item.getReceivedBytes()
      entry.totalBytes = item.getTotalBytes()
      entry.state = state === 'completed' ? 'complete' : 'interrupted'
      downloadControllers.delete(entry.id)
      scheduleDesktopStateSave()
      agentView?.webContents.send('agent:download-changed', entry)
    })
  })
}

function currentTab() {
  const currentId = activeTabByWindow.get(activeWindowId) ?? activeTabId
  const active = browserTabs.get(currentId)
  const windowTabs = [...browserTabs.values()].filter((tab) => tab.windowId === (active?.windowId ?? activeWindowId))
  return {
    id: active?.id ?? 0,
    index: Math.max(0, windowTabs.findIndex((tab) => tab.id === active?.id)),
    windowId: active?.windowId ?? activeWindowId,
    active: true,
    pinned: false,
    highlighted: true,
    incognito: false,
    url: active?.state.url ?? latestBrowserState.url,
    title: active?.state.title ?? latestBrowserState.title,
    status: active?.state.loading ? 'loading' : 'complete',
    audible: false,
    mutedInfo: { muted: false },
    groupId: active?.groupId ?? -1,
  }
}

function chromeTab(tab: BrowserTab, index: number) {
  const activeId = activeTabByWindow.get(tab.windowId)
  return {
    id: tab.id,
    index,
    windowId: tab.windowId,
    active: tab.id === activeId,
    pinned: false,
    highlighted: tab.id === activeId,
    incognito: false,
    url: tab.state.url,
    title: tab.state.title,
    favIconUrl: tab.state.faviconUrl,
    status: tab.state.loading ? 'loading' : 'complete',
    audible: false,
    mutedInfo: { muted: false },
    groupId: tab.groupId ?? -1,
  }
}

function windowState(windowId: number, populate = false) {
  const target = desktopWindows.get(windowId)
  if (!target || target.isDestroyed()) throw new Error(`Window ${windowId} not found`)
  const [width, height] = target.getSize()
  const [left, top] = target.getPosition()
  const state = target.isFullScreen()
    ? 'fullscreen'
    : target.isMaximized()
      ? 'maximized'
      : target.isMinimized()
        ? 'minimized'
        : 'normal'
  const tabs = [...browserTabs.values()]
    .filter((tab) => tab.windowId === windowId && tab.profileId === activeProfileId)
    .map((tab, index) => chromeTab(tab, index))
  return {
    id: windowId,
    focused: target.isFocused(),
    alwaysOnTop: target.isAlwaysOnTop(),
    type: 'normal',
    state,
    left,
    top,
    width,
    height,
    ...(populate ? { tabs } : {}),
  }
}

function activeTabForWindow(windowId: number): BrowserTab | undefined {
  const activeId = activeTabByWindow.get(windowId)
  return (activeId === undefined ? undefined : browserTabs.get(activeId)) ?? [...browserTabs.values()].find((tab) => tab.windowId === windowId)
}

function tabIndex(tabId: number): number {
  const tab = browserTabs.get(tabId)
  if (!tab) return -1
  return [...browserTabs.values()].filter((candidate) => candidate.windowId === tab.windowId).findIndex((candidate) => candidate.id === tabId)
}

function sendExtensionEvent(channel: string, ...args: unknown[]): void {
  agentView?.webContents.send(channel, ...args)
  appView?.webContents.send(channel, ...args)
}

let agentProgressSequence = 0
function emitAgentProgress(progress: Omit<AgentProgress, 'sequence'>): void {
  const event: AgentProgress = { ...progress, sequence: ++agentProgressSequence }
  mainWindow?.webContents.send('agent:progress', event)
  if (progress.detail) {
    void appendFile(join(getRuntimeDir(), 'agent.log'), `${new Date().toISOString()} Agent progress: ${progress.message} — ${progress.detail}\n`).catch(() => {})
  }
}

function notifyRuntimeLifecycle(): void {
  sendExtensionEvent('agent:runtime-startup')
  if (firstRun) {
    sendExtensionEvent('agent:runtime-installed', { reason: 'install' })
    firstRun = false
    scheduleDesktopStateSave()
  }
}

function matchesChromePattern(value: string, pattern: unknown): boolean {
  if (typeof pattern !== 'string' || !pattern) return true
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replaceAll('*', '.*')
  try {
    return new RegExp(`^${escaped}$`, 'i').test(value)
  } catch {
    return false
  }
}

function storageBytes(keys?: unknown): number {
  const selected = storageResult(keys)
  return new TextEncoder().encode(JSON.stringify(selected)).byteLength
}

function storageResult(keys?: unknown): Record<string, unknown> {
  if (keys === undefined || keys === null) return Object.fromEntries(extensionStorage)
  if (typeof keys === 'string') return extensionStorage.has(keys) ? { [keys]: extensionStorage.get(keys) } : {}
  if (Array.isArray(keys)) {
    return Object.fromEntries(keys.filter((key): key is string => typeof key === 'string' && extensionStorage.has(key)).map((key) => [key, extensionStorage.get(key)]))
  }
  if (typeof keys === 'object') {
    return Object.fromEntries(Object.entries(keys).map(([key, fallback]) => [key, extensionStorage.has(key) ? extensionStorage.get(key) : fallback]))
  }
  return {}
}

function applyContentBounds(): void {
  for (const tab of browserTabs.values()) {
    const targetWindow = desktopWindows.get(tab.windowId)
    if (!targetWindow || targetWindow.isDestroyed()) continue
    const bounds = tab.windowId === 1
      ? contentBounds
      : (() => {
          const [width, height] = targetWindow.getContentSize()
          return { x: 0, y: 0, width, height }
        })()
    tab.view.setBounds(bounds)
    tab.view.setVisible(
      tab.profileId === activeProfileId &&
      activeTabByWindow.get(tab.windowId) === tab.id &&
      (tab.windowId !== 1 || activeSurface === 'browser'),
    )
  }
  appView?.setBounds(contentBounds)
  appView?.setVisible(activeSurface !== 'browser')
}

function activateBrowserTab(tabId: number): BrowserState {
  const tab = browserTabs.get(tabId)
  if (!tab) throw new Error(`Browser tab ${tabId} does not exist`)
  const targetWindow = desktopWindows.get(tab.windowId)
  if (!targetWindow || targetWindow.isDestroyed()) throw new Error(`Window ${tab.windowId} does not exist`)
  activeTabByWindow.set(tab.windowId, tabId)
  activeTabId = tabId
  activeWindowId = tab.windowId
  if (tab.windowId === 1) {
    activeSurface = 'browser'
    mainWindow?.webContents.send('app:surface', activeSurface)
    setAssistantBounds()
  }
  targetWindow.contentView.addChildView(tab.view)
  if (tab.windowId === 1 && agentView) mainWindow?.contentView.addChildView(agentView)
  applyContentBounds()
  latestBrowserState = getBrowserState()
  if (tab.windowId === 1) {
    mainWindow?.webContents.send('browser:state', latestBrowserState)
    mainWindow?.webContents.send('browser:tabs', listBrowserTabs())
  }
  agentView?.webContents.send('agent:tabs-activated', { tabId, windowId: tab.windowId })
  scheduleDesktopStateSave()
  return latestBrowserState
}

async function createBrowserTab(input = DEFAULT_URL, activate = true, windowId = activeWindowId): Promise<BrowserState> {
  const targetWindow = desktopWindows.get(windowId) ?? mainWindow
  if (!targetWindow || targetWindow.isDestroyed()) throw new Error(`Window ${windowId} is not ready`)
  const id = nextTabId++
  ensureProfileSession(activeProfileId)
  await ensureProfileExtension(activeProfileId)
  const view = new WebContentsView({
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      partition: `persist:request-browser-${activeProfileId}`,
    },
  })
  const tab: BrowserTab = {
    id,
    windowId,
    profileId: activeProfileId,
    view,
    state: { id, url: 'about:blank', title: 'New tab', canGoBack: false, canGoForward: false, loading: false },
  }
  browserTabs.set(id, tab)
  attachShortcuts(view.webContents)
  targetWindow.contentView.addChildView(view)
  view.setBackgroundColor('#ffffff')
  view.setBounds(windowId === 1 ? contentBounds : { x: 0, y: 0, ...(() => { const [width, height] = targetWindow.getContentSize(); return { width, height } })() })
  view.webContents.setWindowOpenHandler(({ url }) => {
    void createBrowserTab(url, true, windowId)
    return { action: 'deny' }
  })
  view.webContents.on('context-menu', (_event, params) => {
    const menu = Menu.buildFromTemplate([
      { label: 'Back', enabled: view.webContents.canGoBack(), click: () => view.webContents.goBack() },
      { label: 'Forward', enabled: view.webContents.canGoForward(), click: () => view.webContents.goForward() },
      { label: 'Reload', click: () => view.webContents.reload() },
      { type: 'separator' },
      ...(params.selectionText ? [{ role: 'copy' as const }, { type: 'separator' as const }] : []),
      { label: 'Open link in new tab', visible: Boolean(params.linkURL), click: () => void createBrowserTab(params.linkURL, true, windowId) },
      { label: 'Save image as…', visible: Boolean(params.srcURL && params.mediaType === 'image'), click: () => view.webContents.downloadURL(params.srcURL) },
      { type: 'separator' },
      { label: 'Inspect', click: () => view.webContents.inspectElement(params.x, params.y) },
    ])
    menu.popup({ window: targetWindow })
  })
  const update = (change: Partial<BrowserState>) => {
    tab.state = {
      ...tab.state,
      ...change,
      canGoBack: view.webContents.canGoBack(),
      canGoForward: view.webContents.canGoForward(),
    }
    if (tab.windowId === 1 && id === activeTabId) latestBrowserState = tab.state
    broadcastBrowserState()
  }
  view.webContents.on('did-start-loading', () => update({ loading: true }))
  view.webContents.on('did-stop-loading', () => update({ loading: false }))
  view.webContents.on('did-navigate', (_event, url) => {
    update({ url })
    recordHistory(url, tab.state.title)
    sendExtensionEvent('agent:web-navigation-completed', {
      tabId: id,
      frameId: 0,
      frameType: 'outermost_frame',
      url,
      timeStamp: Date.now(),
    })
  })
  view.webContents.on('did-navigate-in-page', (_event, url) => {
    update({ url })
    sendExtensionEvent('agent:web-navigation-committed', {
      tabId: id,
      frameId: 0,
      frameType: 'outermost_frame',
      url,
      timeStamp: Date.now(),
    })
  })
  view.webContents.on('page-title-updated', (_event, title) => {
    update({ title })
    const entry = historyEntries.find((item) => item.url === tab.state.url)
    if (entry) {
      entry.title = title
      scheduleDesktopStateSave()
    }
  })
  view.webContents.on('page-favicon-updated', (_event, favicons) => update({ faviconUrl: favicons[0] }))
  if (activate || !activeTabForWindow(windowId)) activateBrowserTab(id)
  else applyContentBounds()
  // A restored site can keep navigation open while it waits on network,
  // login, or long-lived page requests. Do not hold desktop initialization on
  // that page; the tab can finish loading and broadcast its state normally.
  void view.webContents.loadURL(normalizeNavigationInput(input)).catch((error) => {
    emitLog(`Browser tab ${id} failed to load (${String(error)})`)
  })
  scheduleDesktopStateSave()
  return tab.state
}

async function closeBrowserTab(tabId: number): Promise<void> {
  const tab = browserTabs.get(tabId)
  if (!tab) return
  const targetWindow = desktopWindows.get(tab.windowId)
  const profileTabs = [...browserTabs.values()].filter((item) => item.profileId === tab.profileId && item.windowId === tab.windowId)
  const profileIndex = profileTabs.findIndex((item) => item.id === tabId)
  targetWindow?.contentView.removeChildView(tab.view)
  tab.view.webContents.close()
  browserTabs.delete(tabId)
  agentView?.webContents.send('agent:tabs-removed', tabId, { windowId: tab.windowId, isWindowClosing: false })
  if (activeTabByWindow.get(tab.windowId) === tabId) {
    const profileTabIds = [...browserTabs.values()].filter((item) => item.profileId === activeProfileId && item.windowId === tab.windowId).map((item) => item.id)
    const replacement = profileTabIds[Math.max(0, profileIndex - 1)] ?? profileTabIds[0]
    if (replacement) activateBrowserTab(replacement)
    else if (tab.windowId === 1) await createBrowserTab(DEFAULT_URL, true, 1)
    else targetWindow?.close()
  }
  if (tab.windowId !== 1 && browserTabs.size > 0 && ![...browserTabs.values()].some((candidate) => candidate.windowId === tab.windowId)) {
    desktopWindows.delete(tab.windowId)
    activeTabByWindow.delete(tab.windowId)
  }
  broadcastBrowserState()
  scheduleDesktopStateSave()
}

async function activateProfile(profileId: string): Promise<void> {
  if (!browserProfiles.some((profile) => profile.id === profileId)) throw new Error('Profile not found')
  activeProfileId = profileId
  const existing = [...browserTabs.values()].find((tab) => tab.profileId === profileId)
  if (existing) activateBrowserTab(existing.id)
  else await createBrowserTab(DEFAULT_URL, true)
  scheduleDesktopStateSave()
  mainWindow?.webContents.send('app:profile', browserProfiles.find((profile) => profile.id === profileId))
}

async function openSurface(surface: AppSurface): Promise<void> {
  const routes: Record<AppSurface, string> = {
    browser: '',
    home: 'home',
    'connect-apps': 'connect-apps',
    workspace: 'workspace',
    bookmarks: 'library/bookmarks',
    history: 'library/history',
    downloads: 'library/downloads',
    scheduled: 'scheduled',
    settings: 'settings/ai',
  }
  return openAppRoute(routes[surface], surface)
}

async function openAppRoute(route: string, surface?: AppSurface): Promise<void> {
  const normalizedRoute = route.replace(/^\/+/, '') || 'home'
  const nextSurface = surface ?? (
    normalizedRoute.startsWith('settings/') ? 'settings' :
      normalizedRoute.startsWith('connect-apps') ? 'connect-apps' :
        normalizedRoute.startsWith('workspace') ? 'workspace' :
          normalizedRoute.startsWith('library/bookmarks') ? 'bookmarks' :
            normalizedRoute.startsWith('library/history') ? 'history' :
              normalizedRoute.startsWith('library/downloads') ? 'downloads' :
                normalizedRoute.startsWith('scheduled') ? 'scheduled' : 'home'
  ) as AppSurface
  activeSurface = nextSurface
  mainWindow?.webContents.send('app:surface', activeSurface)
  setAssistantBounds()
  if (nextSurface === 'browser') {
    activateBrowserTab(activeTabId)
    return
  }
  if (!appView || !agentUiBaseUrl) return
  await appView.webContents.loadURL(`${agentUiBaseUrl}/app.html?desktop=1#/${normalizedRoute}`)
  mainWindow?.contentView.addChildView(appView)
  if (agentView) mainWindow?.contentView.addChildView(agentView)
  applyContentBounds()
}

function setAssistantBounds(): void {
  if (!mainWindow || !agentView) return
  const [width, height] = mainWindow.getContentSize()
  const top = activeSurface === 'browser' ? 96 : 56
  const panelWidth = Math.min(assistantWidth, width)
  const isFloating = assistantMode === 'floating'
  agentView.setBounds({
    x: assistantVisible ? Math.max(0, width - panelWidth - (isFloating ? 16 : 0)) : width,
    y: assistantVisible && isFloating ? top + 12 : top,
    width: assistantVisible ? panelWidth : 0,
    height: assistantVisible ? Math.max(0, height - top - (isFloating ? 24 : 0)) : 0,
  })
  agentView.setVisible(assistantVisible)
  mainWindow.webContents.send('app:assistant-visible', assistantVisible)
  mainWindow.webContents.send('app:assistant-layout', { width: assistantWidth, mode: assistantMode })
}

async function createAgentView(): Promise<void> {
  if (!agentUiBaseUrl || !mainWindow) return
  agentView = new WebContentsView({
    webPreferences: {
      preload: join(__dirname, 'agent-preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  })
  mainWindow.contentView.addChildView(agentView)
  agentView.webContents.setWindowOpenHandler(({ url }) => {
    void createBrowserTab(url, true)
    return { action: 'deny' }
  })
  agentView.webContents.on('will-navigate', (event, url) => {
    if (url.startsWith(agentUiBaseUrl)) return
    event.preventDefault()
    void createBrowserTab(url, true)
  })
  agentView.setBackgroundColor('#11151c')
  agentView.webContents.on('did-fail-load', (_event, errorCode, errorDescription) => emitLog(`Request Browser UI failed to load (${errorCode}): ${errorDescription}`))
  agentView.webContents.on('console-message', (_event, level, message) => emitLog(`Request Browser UI: ${message}`))
  agentView.webContents.once('did-finish-load', () => {
    setTimeout(notifyRuntimeLifecycle, 0)
  })
  // Do not block the entire desktop startup on the assistant bundle. Some
  // Chromium pages keep a long-lived request or extension initialization task
  // open even after the document is usable. The browser shell and local agent
  // server must still come up in that case.
  void agentView.webContents.loadURL(`${agentUiBaseUrl}/sidepanel.html?desktop=1`)
    .then(() => setAssistantBounds())
    .catch((error) => emitLog(`Request Browser UI failed to load (${String(error)})`))
  setAssistantBounds()
}

async function createAppView(): Promise<void> {
  if (!agentUiBaseUrl || !mainWindow) return
  appView = new WebContentsView({
    webPreferences: {
      preload: join(__dirname, 'agent-preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  })
  mainWindow.contentView.addChildView(appView)
  appView.webContents.setWindowOpenHandler(({ url }) => {
    void createBrowserTab(url, true)
    return { action: 'deny' }
  })
  appView.webContents.on('will-navigate', (event, url) => {
    if (url.startsWith(agentUiBaseUrl)) return
    event.preventDefault()
    void createBrowserTab(url, true)
  })
  appView.setBackgroundColor('#0d1015')
  appView.setVisible(false)
  appView.webContents.on('did-fail-load', (_event, code, description) => emitLog(`Workspace UI failed to load (${code}): ${description}`))
  appView.webContents.on('did-finish-load', () => {
    setTimeout(notifyRuntimeLifecycle, 0)
  })
}

function registerDesktopWindow(windowId: number, target: BrowserWindow): void {
  desktopWindows.set(windowId, target)
  target.on('focus', () => {
    activeWindowId = windowId
    const active = activeTabForWindow(windowId)
    if (active) activeTabId = active.id
    sendExtensionEvent('agent:window-focus-changed', windowId)
    if (windowId === 1) {
      latestBrowserState = getBrowserState()
      mainWindow?.webContents.send('browser:state', latestBrowserState)
      mainWindow?.webContents.send('browser:tabs', listBrowserTabs())
    }
  })
  target.on('blur', () => sendExtensionEvent('agent:window-focus-changed', -1))
  target.on('resize', () => {
    if (windowId === 1) setAssistantBounds()
    applyContentBounds()
    sendExtensionEvent('agent:window-bounds-changed', windowState(windowId))
  })
  target.on('move', () => sendExtensionEvent('agent:window-bounds-changed', windowState(windowId)))
  target.on('closed', () => {
    for (const tab of [...browserTabs.values()].filter((candidate) => candidate.windowId === windowId)) {
      browserTabs.delete(tab.id)
      if (!tab.view.webContents.isDestroyed()) tab.view.webContents.close()
      agentView?.webContents.send('agent:tabs-removed', tab.id, { windowId, isWindowClosing: true })
    }
    for (const [groupId, group] of browserTabGroups) {
      if (group.windowId === windowId) browserTabGroups.delete(groupId)
    }
    desktopWindows.delete(windowId)
    activeTabByWindow.delete(windowId)
    if (windowId === 1) {
      mainWindow = null
      activeWindowId = 1
      if (process.platform !== 'darwin') app.quit()
    } else if (activeWindowId === windowId) {
      activeWindowId = 1
      const active = activeTabForWindow(1)
      if (active) activeTabId = active.id
    }
    sendExtensionEvent('agent:window-removed', windowId)
    scheduleDesktopStateSave()
  })
}

function applyWindowUpdate(target: BrowserWindow, updateInfo: Record<string, unknown>): void {
  if (updateInfo.focused === true) target.focus()
  if (updateInfo.state === 'maximized') target.maximize()
  if (updateInfo.state === 'normal') {
    if (target.isFullScreen()) target.setFullScreen(false)
    target.unmaximize()
  }
  if (updateInfo.state === 'fullscreen') target.setFullScreen(true)
  if (updateInfo.state === 'minimized') target.minimize()
  if (updateInfo.state === 'docked') target.restore()
  const numeric = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value)
  const [currentWidth, currentHeight] = target.getSize()
  const [currentLeft, currentTop] = target.getPosition()
  const width = numeric(updateInfo.width) ? Math.max(320, Math.round(updateInfo.width)) : currentWidth
  const height = numeric(updateInfo.height) ? Math.max(240, Math.round(updateInfo.height)) : currentHeight
  const left = numeric(updateInfo.left) ? Math.round(updateInfo.left) : currentLeft
  const top = numeric(updateInfo.top) ? Math.round(updateInfo.top) : currentTop
  if (numeric(updateInfo.width) || numeric(updateInfo.height) || numeric(updateInfo.left) || numeric(updateInfo.top)) {
    target.setBounds({ x: left, y: top, width, height })
  }
  if (typeof updateInfo.alwaysOnTop === 'boolean') target.setAlwaysOnTop(updateInfo.alwaysOnTop)
}

async function startAgentServer(): Promise<void> {
  const runtimeDir = getRuntimeDir()
  const executionDir = join(runtimeDir, 'execution')
  const configPath = join(runtimeDir, 'desktop-sidecar.json')
  await mkdir(executionDir, { recursive: true })
  await writeFile(
    configPath,
    `${JSON.stringify(
      {
        ports: { server: serverPort, cdp: CDP_PORT },
        directories: { resources: getAgentRoot(), execution: executionDir },
        flags: { allow_remote_in_mcp: false },
        instance: { browseros_version: 'electron-dev', chromium_version: process.versions.chrome },
      },
      null,
      2,
    )}\n`,
  )

  const agentRoot = getAgentRoot()
  const packagedServer = app.isPackaged && process.platform === 'win32'
  const bunCommand = process.env.REQUEST_BROWSER_BUN || (process.platform === 'win32' ? 'npx.cmd' : 'npx')
  const bunArgs = process.env.REQUEST_BROWSER_BUN
    ? ['apps/server/src/index.ts', `--config=${configPath}`]
    : ['--yes', 'bun@1.3.6', 'apps/server/src/index.ts', `--config=${configPath}`]
  const quoteCommandArg = (value: string) => /[\s"]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value
  const commandLine = packagedServer
    ? `${join(process.resourcesPath, 'request-browser-server.exe')} --config=${configPath}`
    : [bunCommand, ...bunArgs].map(quoteCommandArg).join(' ')
  const spawnCommand = packagedServer
    ? join(process.resourcesPath, 'request-browser-server.exe')
    : process.platform === 'win32' ? process.env.ComSpec ?? 'cmd.exe' : bunCommand
  const spawnArgs = packagedServer
    ? [`--config=${configPath}`]
    : process.platform === 'win32' ? ['/d', '/s', '/c', commandLine] : bunArgs

  emitLog(`Starting local agent server with ${commandLine}`)
  const child = spawn(spawnCommand, spawnArgs, {
    cwd: agentRoot,
    env: {
      ...process.env,
      PATH: [process.env.PATH, process.env.APPDATA ? join(process.env.APPDATA, 'npm') : '']
        .filter(Boolean)
        .join(process.platform === 'win32' ? ';' : ':'),
      NODE_ENV: packagedServer ? 'production' : 'development',
      BROWSEROS_CDP_MODE: 'electron',
      BROWSEROS_DIR: join(app.getPath('userData'), 'browseros'),
      BROWSEROS_TRUSTED_ORIGINS: agentUiBaseUrl,
      BROWSEROS_CONFIG_URL: process.env.BROWSEROS_CONFIG_URL ?? 'https://browseros.invalid/api/browseros-server/config',
      POSTHOG_API_KEY: process.env.POSTHOG_API_KEY ?? 'phc_request_browser_disabled',
      SENTRY_DSN: process.env.SENTRY_DSN ?? 'https://request-browser.invalid/1',
    },
    stdio: 'pipe',
    windowsHide: true,
  })
  serverProcess = child
  emitLog(`Local agent server process started (${child.pid ?? 'unknown'})`)
  child.stdout.on('data', (chunk) => emitLog(String(chunk).trim()))
  child.stderr.on('data', (chunk) => emitLog(String(chunk).trim()))
  child.once('error', (error) => emitLog(`Agent server spawn error: ${error.message}`))
  child.once('exit', (code) => {
    serverReady = false
    emitLog(`Agent server exited with code ${code ?? 'unknown'}`)
  })

  // The first development launch may need to download the pinned Bun runtime.
  // Keep waiting long enough for that bootstrap instead of permanently marking
  // the otherwise healthy sidecar as unavailable.
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${serverUrl}/system/health`)
      if (response.ok) {
        serverReady = true
        return
      }
    } catch {
      // The server may still be connecting to Electron CDP.
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 350))
  }
  emitLog('Agent server did not become ready; browser shell remains available.')
}

async function ensureResearchSession(goal: string, conversationId: string): Promise<string | null> {
  const response = await fetch(`${serverUrl}/workspace/sessions`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      goal,
      conversationId,
      status: 'running',
      browserProfileId: activeProfileId,
      plan: [
        { title: 'Understand the goal and plan authorized browser actions', toolCategory: 'analysis' },
        { title: 'Browse authorized sources and collect evidence', toolCategory: 'browser' },
        { title: 'Verify findings and save structured results', toolCategory: 'database' },
      ],
    }),
  })
  if (!response.ok) return null
  const value = await response.json() as { session?: { id?: string } }
  return value.session?.id ?? null
}

function extractAgentText(payload: string): string {
  const textParts: string[] = []
  for (const line of payload.split(/\r?\n/)) {
    if (!line.startsWith('data:')) continue
    const data = line.slice(5).trim()
    if (!data || data === '[DONE]') continue
    try {
      const value = JSON.parse(data) as Record<string, unknown>
      if (value.type === 'reasoning-start' || value.type === 'reasoning-delta' || value.type === 'reasoning-end') continue
      const candidate = value.delta ?? value.text ?? value.content
      if (typeof candidate === 'string') textParts.push(candidate)
    } catch {
      if (data.length > 1) textParts.push(data)
    }
  }
  return textParts.join('') || payload.trim() || 'The agent completed without a text response.'
}

async function readAgentResponseStream(response: Response, conversationId: string, workspaceSessionId: string | null): Promise<string> {
  if (!response.body) return response.text()
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let raw = ''
  let buffer = ''
  let lastStatus = ''
  const emitStatus = (phase: AgentProgress['phase'], message: string, detail?: string) => {
    const key = `${phase}:${message}`
    if (key === lastStatus) return
    lastStatus = key
    emitAgentProgress({ conversationId, phase, message, ...(detail ? { detail } : {}) })
    if (workspaceSessionId) {
      const kind = phase === 'thinking' ? 'reasoning-summary' : phase === 'tool' ? 'tool-call' : 'activity'
      void fetch(`${serverUrl}/workspace/sessions/${workspaceSessionId}/events`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ kind, title: message, detail }),
      }).catch(() => {})
    }
  }
  const processData = (data: string) => {
    if (!data || data === '[DONE]') return
    try {
      const value = JSON.parse(data) as Record<string, unknown>
      const type = typeof value.type === 'string' ? value.type : ''
      if (/reasoning-(start|delta)/i.test(type)) emitStatus('thinking', 'Thinking through the goal')
      else if (/tool-(call|input|result)|data-acp-status/i.test(type)) emitStatus('tool', 'Using browser tools')
      else if (/finish|done|complete/i.test(type)) emitStatus('verifying', 'Verifying the result')
      else if (/start|message/i.test(type)) emitStatus('planning', 'Planning the next step')
    } catch {
      // Preserve unparsed data for the final response extractor.
    }
  }
  while (true) {
    const { value, done } = await reader.read()
    if (done) break
    const chunk = decoder.decode(value, { stream: true })
    raw += chunk
    buffer += chunk
    const lines = buffer.split(/\r?\n/)
    buffer = lines.pop() ?? ''
    for (const line of lines) {
      if (line.startsWith('data:')) processData(line.slice(5).trim())
    }
  }
  buffer += decoder.decode()
  if (buffer.startsWith('data:')) processData(buffer.slice(5).trim())
  return raw
}

function updateExtensionStorageValue(key: string, value: unknown): void {
  const oldValue = extensionStorage.get(key)
  extensionStorage.set(key, value)
  const changes = { [key]: { oldValue, newValue: value } }
  agentView?.webContents.send('agent:storage-changed', changes, 'local')
  appView?.webContents.send('agent:storage-changed', changes, 'local')
  scheduleDesktopStateSave()
}

function scheduledJobs(): ScheduledJob[] {
  const jobs = extensionStorage.get('local:scheduledJobs')
  return Array.isArray(jobs) ? jobs.filter((job): job is ScheduledJob => Boolean(job && typeof job === 'object' && typeof (job as ScheduledJob).id === 'string' && typeof (job as ScheduledJob).query === 'string')) : []
}

async function runScheduledJob(jobId: string): Promise<{ success: boolean; error?: string }> {
  const job = scheduledJobs().find((candidate) => candidate.id === jobId)
  if (!job) return { success: false, error: `Scheduled job not found: ${jobId}` }
  const currentRuns = extensionStorage.get('local:scheduledJobRuns')
  const runs: ScheduledJobRun[] = Array.isArray(currentRuns) ? currentRuns.filter((run): run is ScheduledJobRun => Boolean(run && typeof run === 'object')) : []
  const run: ScheduledJobRun = { id: crypto.randomUUID(), jobId, startedAt: new Date().toISOString(), status: 'running' }
  updateExtensionStorageValue('local:scheduledJobRuns', [...runs.filter((candidate) => candidate.jobId !== jobId), ...runs.filter((candidate) => candidate.jobId === jobId).slice(-14), run])
  const controller = new AbortController()
  scheduledRunControllers.set(run.id, controller)
  try {
    const response = await fetch(`${serverUrl}/chat`, {
      method: 'POST',
      signal: controller.signal,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        provider: 'opencode-zen',
        providerId: job.providerId ?? 'opencode-zen',
        providerType: 'opencode-zen',
        providerName: 'OpenCode Zen (Free models)',
        model: 'opencode/mimo-v2.5-free',
        conversationId: crypto.randomUUID(),
        message: job.query,
        mode: 'agent',
        origin: 'scheduled-task',
        acpAgentId: 'opencode',
        acpCommand: getOpenCodeAcpCommand(),
        isScheduledTask: true,
        browserContext: {
          windowId: 1,
          activeTab: { id: latestBrowserState.id, url: latestBrowserState.url, title: latestBrowserState.title },
          tabs: listBrowserTabs().map((tab) => ({ id: tab.id, url: tab.url, title: tab.title })),
        },
      }),
    })
    const body = await response.text()
    if (!response.ok) throw new Error(`Scheduled task failed (${response.status}): ${body.slice(0, 400)}`)
    const completedAt = new Date().toISOString()
    const nextRuns = (extensionStorage.get('local:scheduledJobRuns') as ScheduledJobRun[] | undefined) ?? []
    updateExtensionStorageValue('local:scheduledJobRuns', nextRuns.map((candidate) => candidate.id === run.id ? { ...candidate, status: 'completed', completedAt, result: extractAgentText(body) } : candidate))
    const jobs = scheduledJobs()
    updateExtensionStorageValue('local:scheduledJobs', jobs.map((candidate) => candidate.id === jobId ? { ...candidate, lastRunAt: completedAt } : candidate))
    return { success: true }
  } catch (error) {
    const message = controller.signal.aborted ? 'Cancelled by user' : error instanceof Error ? error.message : String(error)
    const nextRuns = (extensionStorage.get('local:scheduledJobRuns') as ScheduledJobRun[] | undefined) ?? []
    updateExtensionStorageValue('local:scheduledJobRuns', nextRuns.map((candidate) => candidate.id === run.id ? { ...candidate, status: 'failed', completedAt: new Date().toISOString(), result: message, error: message } : candidate))
    return { success: false, error: message }
  } finally {
    scheduledRunControllers.delete(run.id)
  }
}

async function fetchMcpToolsForDesktop(): Promise<{ tools: Array<{ name: string; description?: string }>; error?: string }> {
  try {
    const endpoint = `${serverUrl}/mcp`
    const headers = { accept: 'application/json, text/event-stream', 'content-type': 'application/json' }
    const initializeResponse = await fetch(endpoint, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'initialize',
        params: {
          protocolVersion: '2025-11-25',
          capabilities: {},
          clientInfo: { name: 'request-browser-desktop', version: '0.1.0' },
        },
      }),
    })
    if (!initializeResponse.ok) throw new Error(`MCP initialize failed (${initializeResponse.status})`)
    const initialize = await initializeResponse.json() as { result?: { protocolVersion?: string }; error?: { message?: string } }
    if (initialize.error) throw new Error(initialize.error.message ?? 'MCP initialize failed')
    const protocolVersion = initialize.result?.protocolVersion
    const sessionId = initializeResponse.headers.get('mcp-session-id')
    const requestHeaders = {
      ...headers,
      ...(protocolVersion ? { 'mcp-protocol-version': protocolVersion } : {}),
      ...(sessionId ? { 'mcp-session-id': sessionId } : {}),
    }
    await fetch(endpoint, {
      method: 'POST',
      headers: requestHeaders,
      body: JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }),
    })
    const toolsResponse = await fetch(endpoint, {
      method: 'POST',
      headers: requestHeaders,
      body: JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/list' }),
    })
    if (!toolsResponse.ok) throw new Error(`MCP tools/list failed (${toolsResponse.status})`)
    const toolsValue = await toolsResponse.json() as { result?: { tools?: Array<{ name?: unknown; description?: unknown }> }; error?: { message?: string } }
    if (toolsValue.error) throw new Error(toolsValue.error.message ?? 'MCP tools/list failed')
    const tools = (toolsValue.result?.tools ?? []).flatMap((tool) => typeof tool.name === 'string' ? [{ name: tool.name, ...(typeof tool.description === 'string' ? { description: tool.description } : {}) }] : [])
    return { tools }
  } catch (error) {
    return { tools: [], error: error instanceof Error ? error.message : String(error) }
  }
}

async function sendAgentMessage(goal: string, conversationId: string): Promise<AgentResult> {
  if (!serverReady) throw new Error('The local agent server is not ready yet.')
  const workspaceSessionId = await ensureResearchSession(goal, conversationId)
  emitAgentProgress({ conversationId, phase: 'started', message: 'Goal started', detail: goal })
  emitAgentProgress({ conversationId, phase: 'planning', message: 'Planning the next step' })
  if (workspaceSessionId) {
    void fetch(`${serverUrl}/workspace/sessions/${workspaceSessionId}/events`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ kind: 'activity', title: 'Goal delegated to OpenCode', detail: goal }),
    })
  }
  const response = await fetch(`${serverUrl}/chat`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      provider: 'opencode-zen',
      providerId: 'opencode-zen',
      providerType: 'opencode-zen',
      providerName: 'OpenCode Zen (Free models)',
      model: 'opencode/mimo-v2.5-free',
      conversationId,
      message: goal,
      mode: 'agent',
      origin: 'sidepanel',
      acpAgentId: 'opencode',
      acpCommand: getOpenCodeAcpCommand(),
      browserContext: {
        windowId: 1,
        activeTab: { id: latestBrowserState.id, url: latestBrowserState.url, title: latestBrowserState.title },
        tabs: listBrowserTabs().map((tab) => ({ id: tab.id, url: tab.url, title: tab.title })),
      },
    }),
  })
  const body = await readAgentResponseStream(response, conversationId, workspaceSessionId)
  if (!response.ok) {
    emitAgentProgress({ conversationId, phase: 'error', message: 'Goal failed', detail: body.slice(0, 500) })
    throw new Error(`Agent request failed (${response.status}): ${body.slice(0, 500)}`)
  }
  emitAgentProgress({ conversationId, phase: 'verifying', message: 'Verifying the result' })
  if (workspaceSessionId) {
    const sessionResponse = await fetch(`${serverUrl}/workspace/sessions/${workspaceSessionId}`).catch(() => null)
    if (sessionResponse?.ok) {
      const sessionValue = await sessionResponse.json() as { session?: { plan?: Array<{ id: string }> } }
      await Promise.all((sessionValue.session?.plan ?? []).map((step) => fetch(`${serverUrl}/workspace/plan-steps/${step.id}`, {
        method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ status: 'completed' }),
      }).catch(() => undefined)))
    }
    await fetch(`${serverUrl}/workspace/sessions/${workspaceSessionId}`, { method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ status: 'completed' }) }).catch(() => undefined)
    await fetch(`${serverUrl}/workspace/sessions/${workspaceSessionId}/recap`, { method: 'POST' }).catch(() => undefined)
    await fetch(`${serverUrl}/workspace/sessions/${workspaceSessionId}/suggestion`, { method: 'POST' }).catch(() => undefined)
  }
  emitAgentProgress({ conversationId, phase: 'completed', message: 'Goal complete' })
  return { text: extractAgentText(body), sessionId: conversationId, raw: body }
}

async function createWindow(): Promise<void> {
  if (!agentUiServer) await startAgentUiServer()
  if (!agentExtensionRoot) await loadAgentExtension()
  mainWindow = new BrowserWindow({
    width: 1480,
    height: 920,
    minWidth: 1080,
    minHeight: 680,
    backgroundColor: '#0d1015',
    title: 'Request Browser',
    webPreferences: {
      preload: join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  })
  registerDesktopWindow(1, mainWindow)

  await mainWindow.loadFile(join(__dirname, 'renderer/index.html'))
  attachShortcuts(mainWindow.webContents)
  const tabsToRestore = savedBrowserTabs.filter((tab) => tab.profileId === activeProfileId)
  if (tabsToRestore.length) {
    for (const savedTab of tabsToRestore) {
      const restored = await createBrowserTab(savedTab.url, false)
      const restoredTab = browserTabs.get(restored.id)
      if (restoredTab && savedTab.groupId !== undefined) restoredTab.groupId = savedTab.groupId
    }
    const restoredTabs = [...browserTabs.values()].filter((tab) => tab.profileId === activeProfileId)
    const activeIndex = Math.max(0, tabsToRestore.findIndex((tab) => tab.active))
    activateBrowserTab(restoredTabs[activeIndex]?.id ?? restoredTabs[0].id)
  } else {
    await createBrowserTab(DEFAULT_URL, true)
  }
  await createAppView()
  await createAgentView()
  if (!serverProcess) {
    void startAgentServer().catch((error) => emitLog(`Agent server startup error: ${error instanceof Error ? error.stack ?? error.message : String(error)}`))
  }
}

function registerIpc(): void {
  ipcMain.handle('browser:navigate', (_event, input: string) => navigate(input))
  ipcMain.handle('browser:back', async () => {
    browserTabs.get(activeTabId)?.view.webContents.goBack()
    return getBrowserState()
  })
  ipcMain.handle('browser:forward', async () => {
    browserTabs.get(activeTabId)?.view.webContents.goForward()
    return getBrowserState()
  })
  ipcMain.handle('browser:reload', async () => {
    browserTabs.get(activeTabId)?.view.webContents.reload()
    return getBrowserState()
  })
  ipcMain.on('browser:set-bounds', (_event, bounds: BrowserBounds) => {
    contentBounds = {
      x: Math.round(bounds.x),
      y: Math.round(bounds.y),
      width: Math.round(bounds.width),
      height: Math.round(bounds.height),
    }
    applyContentBounds()
  })
  ipcMain.handle('browser:get-state', () => getBrowserState())
  ipcMain.handle('browser:list-tabs', () => listBrowserTabs())
  ipcMain.handle('browser:create-tab', (_event, input?: string) => createBrowserTab(input ?? DEFAULT_URL, true))
  ipcMain.handle('browser:activate-tab', (_event, tabId: number) => activateBrowserTab(tabId))
  ipcMain.handle('browser:close-tab', (_event, tabId: number) => closeBrowserTab(tabId))
  ipcMain.handle('browser:bookmark-current', () => {
    const current = getBrowserState()
    if (!bookmarks.some((bookmark) => bookmark.url === current.url)) {
      bookmarks.unshift({ id: crypto.randomUUID(), url: current.url, title: current.title || current.url, dateAdded: Date.now() })
      scheduleDesktopStateSave()
    }
  })
  ipcMain.handle('browser:devtools', () => browserTabs.get(activeTabId)?.view.webContents.openDevTools({ mode: 'detach' }))
  ipcMain.handle('agent:send', (_event, input: { goal: string; conversationId: string }) => sendAgentMessage(input.goal, input.conversationId))
  ipcMain.handle('workspace:databases', async () => {
    if (!serverReady) return []
    const response = await fetch(`${serverUrl}/workspace/databases`)
    if (!response.ok) return []
    const value = (await response.json()) as { databases?: Array<{ id: string; name: string; recordCount?: number }> }
    return (value.databases ?? []).map((database) => ({ ...database, recordCount: database.recordCount ?? 0 }))
  })
  ipcMain.handle('workspace:sessions', async () => {
    if (!serverReady) return []
    const response = await fetch(`${serverUrl}/workspace/sessions?limit=20`)
    if (!response.ok) return []
    const value = (await response.json()) as { sessions?: Array<{ id: string; goal: string; status: string }> }
    return value.sessions ?? []
  })
  ipcMain.handle('app:server-status', async () => {
    if (!serverReady && serverProcess) {
      try {
        serverReady = (await fetch(`${serverUrl}/system/health`)).ok
      } catch {
        // The renderer polls this status while the local sidecar starts.
      }
    }
    return { ready: serverReady, url: serverUrl }
  })
  ipcMain.handle('app:open-surface', (_event, surface: AppSurface) => openSurface(surface))
  ipcMain.handle('app:show-browser-menu', () => {
    if (!mainWindow) return
    const menu = Menu.buildFromTemplate([
      { label: 'New tab', accelerator: 'CmdOrCtrl+T', click: () => void createBrowserTab(DEFAULT_URL, true) },
      { label: 'New research task', click: () => void openSurface('home') },
      { type: 'separator' },
      { label: 'Bookmark this page', accelerator: 'CmdOrCtrl+D', click: () => {
        const current = getBrowserState()
        if (!bookmarks.some((bookmark) => bookmark.url === current.url)) {
          bookmarks.unshift({ id: crypto.randomUUID(), url: current.url, title: current.title || current.url, dateAdded: Date.now() })
          scheduleDesktopStateSave()
        }
      } },
      { label: 'Bookmarks', click: () => void openSurface('bookmarks') },
      { label: 'History', accelerator: 'CmdOrCtrl+H', click: () => void openSurface('history') },
      { label: 'Downloads', accelerator: 'CmdOrCtrl+J', click: () => void openSurface('downloads') },
      { label: 'Import bookmarks…', click: () => void importBookmarksFromHtml() },
      { type: 'separator' },
      { label: assistantVisible ? 'Hide AI panel' : 'Show AI panel', click: () => {
        assistantVisible = !assistantVisible
        setAssistantBounds()
      } },
      { label: assistantMode === 'floating' ? 'Dock AI panel' : 'Float AI panel', click: () => {
        assistantMode = assistantMode === 'floating' ? 'docked' : 'floating'
        browserPrefs.set('request-browser.ai-panel.mode', assistantMode)
        scheduleDesktopStateSave()
        setAssistantBounds()
      } },
      { label: 'Block common ads and trackers', type: 'checkbox', checked: browserPrefs.get('request-browser.ad-blocking.enabled') === true, click: (item) => {
        browserPrefs.set('request-browser.ad-blocking.enabled', item.checked)
        scheduleDesktopStateSave()
      } },
      { label: 'Settings', click: () => void openSurface('settings') },
      {
        label: 'Zoom',
        submenu: [
          { label: 'Zoom in', accelerator: 'CmdOrCtrl+Plus', click: () => { const target = browserTabs.get(activeTabId)?.view.webContents; if (target) target.setZoomFactor(Math.min(3, target.getZoomFactor() + 0.1)) } },
          { label: 'Zoom out', accelerator: 'CmdOrCtrl+-', click: () => { const target = browserTabs.get(activeTabId)?.view.webContents; if (target) target.setZoomFactor(Math.max(0.25, target.getZoomFactor() - 0.1)) } },
          { label: 'Actual size', accelerator: 'CmdOrCtrl+0', click: () => browserTabs.get(activeTabId)?.view.webContents.setZoomFactor(1) },
        ],
      },
      { label: 'Print…', accelerator: 'CmdOrCtrl+P', click: () => browserTabs.get(activeTabId)?.view.webContents.print() },
      { label: 'Full screen', accelerator: 'F11', click: () => mainWindow?.setFullScreen(!mainWindow.isFullScreen()) },
      { type: 'separator' },
      { label: 'Developer tools', click: () => browserTabs.get(activeTabId)?.view.webContents.openDevTools({ mode: 'detach' }) },
    ])
    menu.popup({ window: mainWindow })
  })
  ipcMain.handle('app:show-profile-menu', () => {
    if (!mainWindow) return
    const menu = Menu.buildFromTemplate([
      ...browserProfiles.map((profile) => ({
        label: profile.name,
        type: 'radio' as const,
        checked: profile.id === activeProfileId,
        click: () => void activateProfile(profile.id),
      })),
      { type: 'separator' },
      { label: 'New profile', click: () => {
        const profile = { id: crypto.randomUUID(), name: `Profile ${browserProfiles.length + 1}`, createdAt: Date.now() }
        browserProfiles.push(profile)
        void activateProfile(profile.id)
      } },
    ])
    menu.popup({ window: mainWindow })
  })
  ipcMain.handle('app:get-active-profile', () => browserProfiles.find((profile) => profile.id === activeProfileId) ?? browserProfiles[0])
  ipcMain.handle('app:get-preferences', () => getDesktopPreferences())
  ipcMain.handle('app:get-assistant-layout', () => ({ width: assistantWidth, mode: assistantMode }))
  ipcMain.handle('app:set-assistant-width', (_event, value: number) => {
    if (typeof value !== 'number' || !Number.isFinite(value)) return { width: assistantWidth }
    assistantWidth = Math.min(MAX_AGENT_PANEL_WIDTH, Math.max(MIN_AGENT_PANEL_WIDTH, Math.round(value)))
    browserPrefs.set('request-browser.ai-panel.width', assistantWidth)
    scheduleDesktopStateSave()
    setAssistantBounds()
    return { width: assistantWidth }
  })
  ipcMain.handle('app:set-assistant-mode', (_event, value: AssistantMode) => {
    if (value !== 'docked' && value !== 'floating') return { mode: assistantMode }
    assistantMode = value
    browserPrefs.set('request-browser.ai-panel.mode', assistantMode)
    scheduleDesktopStateSave()
    setAssistantBounds()
    return { mode: assistantMode }
  })
  ipcMain.handle('agent:assistant-visible', (_event, visible: boolean) => {
    assistantVisible = Boolean(visible)
    setAssistantBounds()
    agentView?.webContents.send(assistantVisible ? 'agent:sidepanel-opened' : 'agent:sidepanel-closed', { windowId: 1, tabId: activeTabId })
  })
  ipcMain.handle('agent:assistant-toggle', () => {
    assistantVisible = !assistantVisible
    setAssistantBounds()
    return { opened: assistantVisible }
  })
  ipcMain.handle('agent:assistant-is-open', () => assistantVisible)
  ipcMain.handle('agent:sidepanel-options', (_event, options: Record<string, unknown> = {}) => {
    if (typeof options.enabled === 'boolean') {
      browserPrefs.set('browseros.side_panel.enabled', options.enabled)
      if (!options.enabled && assistantVisible) {
        assistantVisible = false
        setAssistantBounds()
      }
    }
    if (typeof options.path === 'string') browserPrefs.set('browseros.side_panel.path', options.path)
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.on('agent:runtime-url', (event, path = '') => {
    event.returnValue = `${agentUiBaseUrl}/${String(path).replace(/^\/+/, '')}`
  })
  ipcMain.handle('agent:tabs-query', (_event, queryInfo: Record<string, unknown> = {}) => {
    const tabs = [...browserTabs.values()].filter((tab) => tab.profileId === activeProfileId).map(chromeTab)
    return tabs.filter((tab) => {
      if (queryInfo.active === true && !tab.active) return false
      if (queryInfo.currentWindow === true && tab.windowId !== activeWindowId) return false
      if (typeof queryInfo.windowId === 'number' && tab.windowId !== queryInfo.windowId) return false
      if (typeof queryInfo.status === 'string' && tab.status !== queryInfo.status) return false
      if (typeof queryInfo.groupId === 'number' && tab.groupId !== queryInfo.groupId) return false
      if (typeof queryInfo.title === 'string' && !matchesChromePattern(tab.title, queryInfo.title)) return false
      if (typeof queryInfo.url === 'string' && !matchesChromePattern(tab.url, queryInfo.url)) return false
      if (Array.isArray(queryInfo.url) && queryInfo.url.length > 0 && !queryInfo.url.some((pattern) => matchesChromePattern(tab.url, pattern))) return false
      return true
    })
  })
  ipcMain.handle('agent:tabs-get', (_event, tabId: number) => {
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    return chromeTab(tab, tabIndex(tabId))
  })
  ipcMain.handle('agent:tabs-reload', async (_event, tabId: number, bypassCache = false) => {
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    if (bypassCache) tab.view.webContents.reloadIgnoringCache()
    else tab.view.webContents.reload()
    return chromeTab(tab, tabIndex(tabId))
  })
  ipcMain.handle('agent:tabs-duplicate', async (_event, tabId: number) => {
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    const state = await createBrowserTab(tab.state.url, true, tab.windowId)
    return chromeTab(browserTabs.get(state.id)!, tabIndex(state.id))
  })
  ipcMain.handle('agent:tabs-group', (_event, details: { tabIds?: number | number[]; groupId?: number } = {}) => {
    const tabIds = Array.isArray(details.tabIds) ? details.tabIds : typeof details.tabIds === 'number' ? [details.tabIds] : []
    const tabs = tabIds.map((tabId) => browserTabs.get(tabId)).filter((tab): tab is BrowserTab => Boolean(tab && tab.profileId === activeProfileId))
    if (!tabs.length) throw new Error('No valid tabs supplied for grouping')
    let groupId = details.groupId
    if (typeof groupId !== 'number' || groupId < 0 || !browserTabGroups.has(groupId) || browserTabGroups.get(groupId)?.windowId !== tabs[0].windowId) {
      groupId = nextTabGroupId++
      browserTabGroups.set(groupId, { id: groupId, title: '', color: 'grey', collapsed: false, windowId: tabs[0].windowId })
    }
    for (const tab of tabs) tab.groupId = groupId
    scheduleDesktopStateSave()
    return groupId
  })
  ipcMain.handle('agent:tabs-ungroup', (_event, tabIds: number | number[]) => {
    const ids = Array.isArray(tabIds) ? tabIds : [tabIds]
    const groupsToCheck = new Set<number>()
    for (const tabId of ids) {
      const tab = browserTabs.get(tabId)
      if (tab?.groupId !== undefined) groupsToCheck.add(tab.groupId)
      if (tab) tab.groupId = undefined
    }
    for (const groupId of groupsToCheck) {
      if (![...browserTabs.values()].some((tab) => tab.groupId === groupId)) browserTabGroups.delete(groupId)
    }
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.handle('agent:tabgroups-query', (_event, queryInfo: Record<string, unknown> = {}) => {
    return [...browserTabGroups.values()].filter((group) => {
      if (typeof queryInfo.windowId === 'number' && group.windowId !== queryInfo.windowId) return false
      if (typeof queryInfo.title === 'string' && !matchesChromePattern(group.title, queryInfo.title)) return false
      return true
    }).map((group) => ({ ...group, tabIds: [...browserTabs.values()].filter((tab) => tab.groupId === group.id).map((tab) => tab.id) }))
  })
  ipcMain.handle('agent:tabgroups-get', (_event, groupId: number) => {
    const group = browserTabGroups.get(groupId)
    if (!group) throw new Error(`Tab group ${groupId} not found`)
    return { ...group, tabIds: [...browserTabs.values()].filter((tab) => tab.groupId === groupId).map((tab) => tab.id) }
  })
  ipcMain.handle('agent:tabgroups-update', (_event, groupId: number, updateProperties: Record<string, unknown> = {}) => {
    const group = browserTabGroups.get(groupId)
    if (!group) throw new Error(`Tab group ${groupId} not found`)
    if (typeof updateProperties.title === 'string') group.title = updateProperties.title
    if (typeof updateProperties.color === 'string') group.color = updateProperties.color
    if (typeof updateProperties.collapsed === 'boolean') group.collapsed = updateProperties.collapsed
    scheduleDesktopStateSave()
    return { ...group, tabIds: [...browserTabs.values()].filter((tab) => tab.groupId === groupId).map((tab) => tab.id) }
  })
  ipcMain.handle('agent:tabs-create', async (_event, createProperties: Record<string, unknown>) => {
    if (createProperties.url === 'chrome://settings/importData') {
      await importBookmarksFromHtml()
      return currentTab()
    }
    const targetWindowId = typeof createProperties.windowId === 'number' && desktopWindows.has(createProperties.windowId)
      ? createProperties.windowId
      : activeWindowId
    const state = await createBrowserTab(typeof createProperties.url === 'string' ? createProperties.url : DEFAULT_URL, createProperties.active !== false, targetWindowId)
    return chromeTab(browserTabs.get(state.id)!, tabIndex(state.id))
  })
  ipcMain.handle('agent:tabs-update', async (_event, tabId: number, updateProperties: Record<string, unknown>) => {
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    if (updateProperties.active === true) activateBrowserTab(tabId)
    if (typeof updateProperties.url === 'string') {
      const url = normalizeNavigationInput(updateProperties.url)
      tab.state = { ...tab.state, url, loading: true }
      broadcastBrowserState()
      void tab.view.webContents.loadURL(url).catch((error) => {
        tab.state = { ...tab.state, loading: false }
        broadcastBrowserState()
        emitLog(`Tab ${tabId} navigation failed (${String(error)})`)
      })
    }
    if (typeof updateProperties.muted === 'boolean') tab.view.webContents.setAudioMuted(updateProperties.muted)
    return chromeTab(tab, [...browserTabs.keys()].indexOf(tabId))
  })
  ipcMain.handle('agent:tabs-remove', (_event, tabId: number) => closeBrowserTab(tabId).then(() => true))
  ipcMain.handle('agent:tabs-send-message', async (_event, tabId: number, message: unknown) => {
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    // Electron's WebContentsView tabs are not Chrome extension tabs, so the
    // native tabs.sendMessage transport is unavailable. Forward the message
    // through the page world as a narrowly-scoped event; Request Browser
    // content scripts listen for this bridge in addition to Chrome messaging.
    const serializedMessage = JSON.stringify(message ?? null)
    await tab.view.webContents.executeJavaScript(
      `window.postMessage({source:"request-browser",message:${serializedMessage}},"*")`,
      true,
    )
    return { res: undefined }
  })
  ipcMain.handle('agent:scripting-execute-script', async (_event, details: { target?: { tabId?: number }; func?: string; args?: unknown[]; code?: string }) => {
    const tabId = details.target?.tabId ?? activeTabId
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    const args = JSON.stringify(details.args ?? [])
    const source = details.func
      ? `(async()=>{const fn=(${details.func});return await fn(...${args})})()`
      : details.code
        ? `(async()=>{${details.code}\n})()`
        : 'undefined'
    const result = await tab.view.webContents.executeJavaScript(source, true)
    return [{ result }]
  })
  ipcMain.handle('agent:scripting-insert-css', async (_event, details: { target?: { tabId?: number }; css?: string }) => {
    const tabId = details.target?.tabId ?? activeTabId
    const tab = browserTabs.get(tabId)
    if (!tab || typeof details.css !== 'string') throw new Error(`Tab ${tabId} not found or CSS is missing`)
    const key = await tab.view.webContents.insertCSS(details.css)
    const styles = injectedStyles.get(tabId) ?? new Map<string, string>()
    styles.set(key, details.css)
    injectedStyles.set(tabId, styles)
    return key
  })
  ipcMain.handle('agent:scripting-remove-css', async (_event, tabId: number, key: string) => {
    const tab = browserTabs.get(tabId)
    if (!tab) throw new Error(`Tab ${tabId} not found`)
    await tab.view.webContents.removeInsertedCSS(key)
    injectedStyles.get(tabId)?.delete(key)
    return true
  })
  ipcMain.handle('agent:window-current', () => windowState(activeWindowId))
  ipcMain.handle('agent:window-get', (_event, windowId: number, getInfo: Record<string, unknown> = {}) => windowState(windowId, getInfo.populate === true))
  ipcMain.handle('agent:window-all', (_event, getInfo: Record<string, unknown> = {}) => [...desktopWindows.keys()].map((windowId) => windowState(windowId, getInfo.populate === true)))
  ipcMain.handle('agent:window-create', async (_event, createData: { url?: string; focused?: boolean; width?: number; height?: number; left?: number; top?: number; state?: string } = {}) => {
    const windowId = nextWindowId++
    const target = new BrowserWindow({
      width: typeof createData.width === 'number' ? Math.max(320, Math.round(createData.width)) : 1280,
      height: typeof createData.height === 'number' ? Math.max(240, Math.round(createData.height)) : 800,
      x: typeof createData.left === 'number' ? Math.round(createData.left) : undefined,
      y: typeof createData.top === 'number' ? Math.round(createData.top) : undefined,
      show: false,
      backgroundColor: '#ffffff',
      title: 'Request Browser',
      webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true },
    })
    registerDesktopWindow(windowId, target)
    const state = await createBrowserTab(createData.url ?? DEFAULT_URL, true, windowId)
    if (createData.state === 'maximized') target.maximize()
    if (createData.state === 'fullscreen') target.setFullScreen(true)
    if (createData.focused !== false) target.show()
    else target.showInactive()
    sendExtensionEvent('agent:window-created', windowState(windowId, true))
    return windowState(windowId, true)
  })
  ipcMain.handle('agent:window-update', async (_event, windowId: number, updateInfo: Record<string, unknown> = {}) => {
    const target = desktopWindows.get(windowId)
    if (!target || target.isDestroyed()) throw new Error(`Window ${windowId} not found`)
    applyWindowUpdate(target, updateInfo)
    applyContentBounds()
    return windowState(windowId, updateInfo.populate === true)
  })
  ipcMain.handle('agent:window-remove', (_event, windowId: number) => {
    const target = desktopWindows.get(windowId)
    if (!target || target.isDestroyed()) return false
    target.close()
    return true
  })
  ipcMain.handle('agent:runtime-message', async (_event, message: unknown) => {
    let response: unknown
    if (message && typeof message === 'object') {
      const value = message as { type?: string; data?: Record<string, unknown> }
      if (value.type === 'runtime.getTabId') response = { tabId: activeTabId }
      if (value.type === 'checkHealth') {
        try {
          response = { healthy: (await fetch(`${serverUrl}/system/health`)).ok }
        } catch {
          response = { healthy: false }
        }
      }
      if (value.type === 'fetchMcpTools') response = await fetchMcpToolsForDesktop()
      if (value.type === 'runScheduledJob' && typeof value.data?.jobId === 'string') response = await runScheduledJob(value.data.jobId)
      if (value.type === 'cancelScheduledJobRun' && typeof value.data?.runId === 'string') {
        const controller = scheduledRunControllers.get(value.data.runId)
        response = controller ? (controller.abort(), { success: true }) : { success: false, error: 'Run not found or already completed' }
      }
      if (value.type === 'runtime.authSuccess') {
        response = { success: true }
        void openAppRoute('home', 'home')
      }
      if (value.type === 'runtime.stopAgent') {
        extensionStorage.set('stop-agent', { ...(value.data ?? {}), timestamp: Date.now() })
        scheduleDesktopStateSave()
        response = { success: true }
      }
      if (value.type === 'runtime.actionClicked') {
        assistantVisible = !assistantVisible
        setAssistantBounds()
        sendExtensionEvent('agent:action-clicked', currentTab())
        response = { opened: assistantVisible }
      }
      if (value.type === 'open') {
        extensionStorage.set('search-actions', value.data ?? {})
        assistantVisible = true
        setAssistantBounds()
        response = { opened: true }
      }
      if (value.type === 'runtime.sidePanelScopeChanged' && typeof value.data?.perWindow === 'boolean') {
        browserPrefs.set('browseros.side_panel.per_window', value.data.perWindow)
        scheduleDesktopStateSave()
      }
    }
    agentView?.webContents.send('agent:runtime-message', message, { id: 1, url: latestBrowserState.url })
    appView?.webContents.send('agent:runtime-message', message, { id: 1, url: latestBrowserState.url })
    // @webext-core/messaging expects a { res, err } envelope. Returning the
    // raw value made every unhandled BrowserOS runtime message look like a
    // transport failure to the settings and scheduling screens.
    return { res: response }
  })
  ipcMain.handle('agent:storage-get', (_event, keys?: unknown) => storageResult(keys))
  ipcMain.handle('agent:storage-bytes', (_event, keys?: unknown) => storageBytes(keys))
  ipcMain.handle('agent:storage-set', async (_event, items: Record<string, unknown>) => {
    const changes: Record<string, { oldValue?: unknown; newValue?: unknown }> = {}
    for (const [key, value] of Object.entries(items ?? {})) {
      changes[key] = { oldValue: extensionStorage.get(key), newValue: value }
      extensionStorage.set(key, value)
    }
    agentView?.webContents.send('agent:storage-changed', changes, 'local')
    appView?.webContents.send('agent:storage-changed', changes, 'local')
    scheduleDesktopStateSave()
    if ('llm-providers' in items) {
      migrateOpenCodeProviders()
      await syncOpenCodeCredentials()
    }
    if ('llm-providers' in items && serverProcess) {
      stopAgentServer()
      setTimeout(() => void startAgentServer().catch((error) => emitLog(`Agent server restart failed: ${String(error)}`)), 500)
    }
  })
  ipcMain.handle('agent:storage-remove', (_event, keys: string | string[]) => {
    const changes: Record<string, { oldValue?: unknown; newValue?: unknown }> = {}
    for (const key of Array.isArray(keys) ? keys : [keys]) {
      if (!extensionStorage.has(key)) continue
      changes[key] = { oldValue: extensionStorage.get(key) }
      extensionStorage.delete(key)
    }
    agentView?.webContents.send('agent:storage-changed', changes, 'local')
    appView?.webContents.send('agent:storage-changed', changes, 'local')
    scheduleDesktopStateSave()
  })
  ipcMain.handle('agent:storage-clear', () => {
    const changes = Object.fromEntries([...extensionStorage.entries()].map(([key, value]) => [key, { oldValue: value }]))
    extensionStorage.clear()
    agentView?.webContents.send('agent:storage-changed', changes, 'local')
    appView?.webContents.send('agent:storage-changed', changes, 'local')
    scheduleDesktopStateSave()
  })
  ipcMain.handle('agent:pref-get', (_event, name: string) => ({ key: name, type: typeof browserPrefs.get(name), value: browserPrefs.get(name) }))
  ipcMain.handle('agent:pref-set', async (_event, name: string, value: unknown) => {
    const isPortPreference = name === 'browseros.server.server_port' || name === 'browseros.server.mcp_port' || name === 'browseros.server.proxy_port'
    const restartRequested = name === 'browseros.server.restart_requested' && value === true
    if (isPortPreference) {
      if (typeof value !== 'number' || !Number.isInteger(value) || value < 1024 || value > 65535) return false
      serverPort = value
      serverUrl = `http://127.0.0.1:${serverPort}`
      browserPrefs.set('browseros.server.server_port', value)
      browserPrefs.set('browseros.server.mcp_port', value)
      browserPrefs.set('browseros.server.proxy_port', value)
    } else {
      browserPrefs.set(name, value)
    }
    scheduleDesktopStateSave()
    broadcastDesktopPreferences()
    if ((isPortPreference || restartRequested) && serverProcess) {
      stopAgentServer()
      setTimeout(() => void startAgentServer().catch((error) => emitLog(`Agent server restart failed: ${String(error)}`)), 350)
    }
    return true
  })
  ipcMain.handle('agent:metric', (_event, eventName: string, properties?: unknown) => {
    emitLog(`Metric ${eventName}${properties ? ` ${JSON.stringify(properties)}` : ''}`)
  })
  ipcMain.handle('agent:choose-path', async (_event, options: { type?: 'file' | 'folder'; title?: string; startingDirectory?: string } = {}) => {
    if (!mainWindow) return null
    const result = await dialog.showOpenDialog(mainWindow, {
      title: options.title,
      defaultPath: options.startingDirectory,
      properties: options.type === 'file' ? ['openFile'] : ['openDirectory', 'createDirectory'],
    })
    if (result.canceled || !result.filePaths[0]) return null
    return { path: result.filePaths[0], name: basename(result.filePaths[0]) }
  })
  ipcMain.handle('agent:open-options', () => {
    return openSurface('settings')
  })
  ipcMain.handle('agent:open-route', (_event, route: string) => {
    const normalizedRoute = String(route ?? '').replace(/^\/+/, '')
    const routeKey = normalizedRoute.split('?')[0]
    const allowedRoutes = new Set([
      'home',
      'connect-apps',
      'workspace',
      'library/bookmarks',
      'library/history',
      'library/downloads',
      'scheduled',
      'settings',
      'settings/ai',
      'settings/chat',
      'settings/mcp',
      'settings/customization',
      'settings/usage',
      'settings/survey',
      'onboarding',
      'onboarding/features',
    ])
    if (!allowedRoutes.has(routeKey)) return undefined
    return openAppRoute(routeKey === 'settings' ? 'settings/ai' : normalizedRoute)
  })
  ipcMain.handle('agent:open-surface', (_event, surface: string) => {
    const allowed: AppSurface[] = ['home', 'connect-apps', 'workspace', 'bookmarks', 'history', 'downloads', 'scheduled', 'settings']
    if (!allowed.includes(surface as AppSurface)) return undefined
    return openSurface(surface as AppSurface)
  })
  ipcMain.handle('agent:alarm-create', (_event, name: string, info: { when?: number; delayInMinutes?: number; periodInMinutes?: number } = {}) => {
    const alarm: AlarmEntry = {
      name,
      scheduledTime: info.when ?? Date.now() + (info.delayInMinutes ?? info.periodInMinutes ?? 1) * 60_000,
      ...(info.periodInMinutes ? { periodInMinutes: info.periodInMinutes } : {}),
    }
    alarms.set(name, alarm)
    scheduleAlarm(alarm)
    scheduleDesktopStateSave()
  })
  ipcMain.handle('agent:alarm-clear', (_event, name: string) => {
    const existed = alarms.delete(name)
    const timer = alarmTimers.get(name)
    if (timer) clearTimeout(timer)
    alarmTimers.delete(name)
    scheduleDesktopStateSave()
    return existed
  })
  ipcMain.handle('agent:alarm-get', (_event, name: string) => alarms.get(name))
  ipcMain.handle('agent:alarm-get-all', () => [...alarms.values()])
  ipcMain.handle('agent:alarm-clear-all', () => {
    for (const timer of alarmTimers.values()) clearTimeout(timer)
    alarms.clear()
    alarmTimers.clear()
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.handle('agent:bookmark-search', (_event, query?: string | { query?: string; url?: string; title?: string }) => {
    const term = typeof query === 'string' ? query : query?.query ?? query?.url ?? query?.title ?? ''
    const normalized = term.toLowerCase()
    return bookmarks.filter((bookmark) => !normalized || `${bookmark.title} ${bookmark.url}`.toLowerCase().includes(normalized))
  })
  ipcMain.handle('agent:bookmark-create', (_event, details: { url?: string; title?: string }) => {
    const url = details.url || latestBrowserState.url
    const existing = bookmarks.find((bookmark) => bookmark.url === url)
    if (existing) return existing
    const bookmark = { id: crypto.randomUUID(), url, title: details.title || latestBrowserState.title || url, dateAdded: Date.now() }
    bookmarks.unshift(bookmark)
    scheduleDesktopStateSave()
    return bookmark
  })
  ipcMain.handle('agent:bookmark-remove', (_event, id: string) => {
    const index = bookmarks.findIndex((bookmark) => bookmark.id === id)
    if (index >= 0) bookmarks.splice(index, 1)
    scheduleDesktopStateSave()
  })
  ipcMain.handle('agent:bookmark-update', (_event, id: string, changes: { title?: string; url?: string } = {}) => {
    const bookmark = bookmarks.find((item) => item.id === id)
    if (!bookmark) return undefined
    if (typeof changes.title === 'string' && changes.title.trim()) bookmark.title = changes.title.trim()
    if (typeof changes.url === 'string' && /^https?:/i.test(changes.url)) bookmark.url = changes.url
    scheduleDesktopStateSave()
    return bookmark
  })
  ipcMain.handle('agent:bookmark-get', (_event, id: string) => bookmarks.find((bookmark) => bookmark.id === id))
  ipcMain.handle('agent:bookmark-tree', () => [{ id: '0', title: '', url: '', children: bookmarks.slice() }])
  ipcMain.handle('agent:history-search', (_event, query: { text?: string; startTime?: number; endTime?: number; maxResults?: number } = {}) => {
    const term = (query.text ?? '').toLowerCase()
    return historyEntries.filter((entry) => {
      if (query.startTime && entry.lastVisitTime < query.startTime) return false
      if (query.endTime && entry.lastVisitTime > query.endTime) return false
      return !term || `${entry.title} ${entry.url}`.toLowerCase().includes(term)
    }).slice(0, query.maxResults ?? 100)
  })
  ipcMain.handle('agent:history-delete-all', () => {
    historyEntries.splice(0, historyEntries.length)
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.handle('agent:history-delete-url', (_event, url: string) => {
    const before = historyEntries.length
    for (let index = historyEntries.length - 1; index >= 0; index -= 1) {
      if (historyEntries[index].url === url) historyEntries.splice(index, 1)
    }
    scheduleDesktopStateSave()
    return before !== historyEntries.length
  })
  ipcMain.handle('agent:history-add-url', (_event, details: { url: string; title?: string }) => {
    if (!/^https?:/i.test(details.url)) return false
    recordHistory(details.url, details.title ?? details.url)
    return true
  })
  ipcMain.handle('agent:history-delete-range', (_event, details: { startTime?: number; endTime?: number } = {}) => {
    const before = historyEntries.length
    for (let index = historyEntries.length - 1; index >= 0; index -= 1) {
      const entry = historyEntries[index]
      if (details.startTime && entry.lastVisitTime < details.startTime) continue
      if (details.endTime && entry.lastVisitTime > details.endTime) continue
      historyEntries.splice(index, 1)
    }
    scheduleDesktopStateSave()
    return before - historyEntries.length
  })
  ipcMain.handle('agent:history-visits', (_event, url: string) => {
    const entry = historyEntries.find((item) => item.url === url)
    return entry ? [{ id: entry.id, visitTime: entry.lastVisitTime, referringVisitId: 0, transition: 'link' }] : []
  })
  ipcMain.handle('agent:top-sites', () => historyEntries.slice().sort((a, b) => b.visitCount - a.visitCount || b.lastVisitTime - a.lastVisitTime).slice(0, 12).map((entry) => ({ title: entry.title, url: entry.url })))
  ipcMain.handle('agent:permissions-contains', (_event, request: { permissions?: string[] } = {}) => (request.permissions ?? []).every((permission) => grantedPermissions.has(permission)))
  ipcMain.handle('agent:permissions-request', (_event, request: { permissions?: string[] } = {}) => {
    for (const permission of request.permissions ?? []) grantedPermissions.add(permission)
    sendExtensionEvent('agent:permission-added', request)
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.handle('agent:permissions-remove', (_event, request: { permissions?: string[] } = {}) => {
    for (const permission of request.permissions ?? []) grantedPermissions.delete(permission)
    sendExtensionEvent('agent:permission-removed', request)
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.handle('agent:download', (_event, options: { url?: string }) => {
    if (!options?.url) throw new Error('A download URL is required')
    browserTabs.get(activeTabId)?.view.webContents.downloadURL(options.url)
    return Date.now()
  })
  ipcMain.handle('agent:downloads-search', (_event, query: { query?: string; limit?: number } = {}) => {
    const term = (query.query ?? '').toLowerCase()
    return downloadEntries.filter((entry) => !term || `${entry.filename} ${entry.url}`.toLowerCase().includes(term)).slice(0, query.limit ?? 100)
  })
  ipcMain.handle('agent:downloads-open', async (_event, id: number | string) => {
    const entry = downloadEntries.find((item) => item.id === String(id))
    if (!entry?.path) return false
    const error = await shell.openPath(entry.path)
    return !error
  })
  ipcMain.handle('agent:downloads-show', (_event, id: number | string) => {
    const entry = downloadEntries.find((item) => item.id === String(id))
    if (!entry?.path) return false
    shell.showItemInFolder(entry.path)
    return true
  })
  ipcMain.handle('agent:downloads-erase', (_event, id?: number | string) => {
    if (id === undefined) downloadEntries.splice(0, downloadEntries.length)
    else {
      const index = downloadEntries.findIndex((item) => item.id === String(id))
      if (index >= 0) downloadEntries.splice(index, 1)
    }
    scheduleDesktopStateSave()
    return true
  })
  ipcMain.handle('agent:downloads-cancel', (_event, id: number | string) => {
    downloadControllers.get(String(id))?.cancel()
    return true
  })
  ipcMain.handle('agent:downloads-pause', (_event, id: number | string) => {
    downloadControllers.get(String(id))?.pause()
    return true
  })
  ipcMain.handle('agent:downloads-resume', (_event, id: number | string) => {
    downloadControllers.get(String(id))?.resume()
    return true
  })
  ipcMain.handle('agent:downloads-remove-file', (_event, id: number | string) => {
    downloadControllers.get(String(id))?.removeFile()
    return true
  })
}

app.whenReady().then(async () => {
  await ensureOpenCodeLauncher()
  await loadDesktopState()
  migrateOpenCodeProviders()
  scheduleDesktopStateSave()
  await syncOpenCodeCredentials()
  restoreAlarms()
  registerIpc()
  await createWindow()
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) void createWindow()
  })
})

app.on('window-all-closed', () => {
  stopAgentServer()
  agentUiServer?.close()
  if (process.platform !== 'darwin') app.quit()
})

app.on('before-quit', () => {
  stopAgentServer()
  agentUiServer?.close()
  if (safeStorage.isEncryptionAvailable()) {
    // Touch the OS-backed secret store so future BYOK credentials use it.
    safeStorage.encryptString('request-browser-ready')
  }
})
