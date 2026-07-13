import { contextBridge, ipcRenderer } from 'electron'

type Listener = (...args: any[]) => void

function eventChannel(channel: string) {
  const listeners = new Map<Listener, (_event: Electron.IpcRendererEvent, ...args: any[]) => void>()
  return {
    addListener(listener: Listener) {
      const handler = (_event: Electron.IpcRendererEvent, ...args: any[]) => listener(...args)
      listeners.set(listener, handler)
      ipcRenderer.on(channel, handler)
    },
    removeListener(listener: Listener) {
      const handler = listeners.get(listener)
      if (handler) ipcRenderer.removeListener(channel, handler)
      listeners.delete(listener)
    },
  }
}

const tabs = {
  query: (queryInfo: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:tabs-query', queryInfo),
  get: (tabId: number) => ipcRenderer.invoke('agent:tabs-get', tabId),
  getCurrent: () => ipcRenderer.invoke('agent:tabs-query', { active: true, currentWindow: true }).then((items: unknown[]) => items[0]),
  create: (createProperties: Record<string, unknown>) => ipcRenderer.invoke('agent:tabs-create', createProperties),
  update: (tabId: number, updateProperties: Record<string, unknown>) => ipcRenderer.invoke('agent:tabs-update', tabId, updateProperties),
  reload: (tabId: number, reloadProperties: { bypassCache?: boolean } = {}) => ipcRenderer.invoke('agent:tabs-reload', tabId, Boolean(reloadProperties.bypassCache)),
  duplicate: (tabId: number) => ipcRenderer.invoke('agent:tabs-duplicate', tabId),
  group: (details: { tabIds?: number | number[]; groupId?: number } = {}) => ipcRenderer.invoke('agent:tabs-group', details),
  ungroup: (tabIds: number | number[]) => ipcRenderer.invoke('agent:tabs-ungroup', tabIds),
  remove: (tabId: number) => ipcRenderer.invoke('agent:tabs-remove', tabId),
  sendMessage: (tabId: number, message: unknown) => ipcRenderer.invoke('agent:tabs-send-message', tabId, message),
  onActivated: eventChannel('agent:tabs-activated'),
  onUpdated: eventChannel('agent:tabs-updated'),
  onRemoved: eventChannel('agent:tabs-removed'),
}

const chromeShim = {
  action: {
    onClicked: eventChannel('agent:action-clicked'),
  },
  runtime: {
    id: 'request-browser-electron',
    getURL: (path = '') => ipcRenderer.sendSync('agent:runtime-url', path),
    getManifest: () => ({
      name: 'Request Browser',
      description: 'AI-first authenticated research browser',
      version: '0.1.0',
      manifest_version: 3,
      permissions: ['topSites', 'storage', 'unlimitedStorage', 'scripting', 'tabs', 'tabGroups', 'sidePanel', 'bookmarks', 'history', 'browserOS', 'alarms', 'webNavigation', 'downloads'],
      host_permissions: ['http://127.0.0.1/*'],
      chrome_url_overrides: { newtab: 'app.html' },
      options_ui: { page: 'app.html#/settings', open_in_tab: true },
      action: { default_title: 'Ask Request Browser' },
      side_panel: { default_path: 'sidepanel.html' },
      background: { service_worker: 'request-browser-background.js' },
      content_scripts: [
        { matches: ['*://*.google.com/*'], js: ['content-scripts/content.js'] },
        { matches: ['*://*/*'], js: ['content-scripts/glow.js'], run_at: 'document_start' },
        { matches: ['*://*/*'], js: ['content-scripts/selection.js'], run_at: 'document_idle' },
      ],
    }),
    openOptionsPage: () => ipcRenderer.invoke('agent:open-options'),
    openSurface: (surface: string) => ipcRenderer.invoke('agent:open-surface', surface),
    openRoute: (route: string) => ipcRenderer.invoke('agent:open-route', route),
    sendMessage: (message: unknown) => ipcRenderer.invoke('agent:runtime-message', message),
    onMessage: eventChannel('agent:runtime-message'),
    onStartup: eventChannel('agent:runtime-startup'),
    onInstalled: eventChannel('agent:runtime-installed'),
    lastError: undefined,
    OnInstalledReason: { INSTALL: 'install', UPDATE: 'update' },
  },
  storage: {
    local: {
      get: (keys?: unknown) => ipcRenderer.invoke('agent:storage-get', keys),
      set: (items: Record<string, unknown>) => ipcRenderer.invoke('agent:storage-set', items),
      remove: (keys: string | string[]) => ipcRenderer.invoke('agent:storage-remove', keys),
      clear: () => ipcRenderer.invoke('agent:storage-clear'),
      getBytesInUse: (keys?: unknown) => ipcRenderer.invoke('agent:storage-bytes', keys),
      onChanged: eventChannel('agent:storage-changed'),
    },
    sync: {
      get: (keys?: unknown) => ipcRenderer.invoke('agent:storage-get', keys),
      set: (items: Record<string, unknown>) => ipcRenderer.invoke('agent:storage-set', items),
    },
    session: {
      get: (keys?: unknown) => ipcRenderer.invoke('agent:storage-get', keys),
      set: (items: Record<string, unknown>) => ipcRenderer.invoke('agent:storage-set', items),
      remove: (keys: string | string[]) => ipcRenderer.invoke('agent:storage-remove', keys),
      clear: () => ipcRenderer.invoke('agent:storage-clear'),
      onChanged: eventChannel('agent:storage-changed'),
    },
  },
  tabs,
  windows: {
    getCurrent: () => ipcRenderer.invoke('agent:window-current'),
    getLastFocused: () => ipcRenderer.invoke('agent:window-current'),
    get: (windowId: number, getInfo: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:window-get', windowId, getInfo),
    getAll: (getInfo: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:window-all', getInfo),
    create: (createData: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:window-create', createData),
    update: (windowId: number, updateInfo: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:window-update', windowId, updateInfo),
    remove: (windowId: number) => ipcRenderer.invoke('agent:window-remove', windowId),
    onCreated: eventChannel('agent:window-created'),
    onRemoved: eventChannel('agent:window-removed'),
    onFocusChanged: eventChannel('agent:window-focus-changed'),
    onBoundsChanged: eventChannel('agent:window-bounds-changed'),
  },
  sidePanel: {
    setOptions: (options: Record<string, unknown>) => ipcRenderer.invoke('agent:sidepanel-options', options),
    open: () => ipcRenderer.invoke('agent:assistant-visible', true),
    close: () => ipcRenderer.invoke('agent:assistant-visible', false),
    browserosToggle: () => ipcRenderer.invoke('agent:assistant-toggle'),
    browserosIsOpen: () => ipcRenderer.invoke('agent:assistant-is-open'),
    onOpened: eventChannel('agent:sidepanel-opened'),
    onClosed: eventChannel('agent:sidepanel-closed'),
  },
  browserOS: {
    // Report the inherited BrowserOS compatibility level so production
    // feature gates do not disable surfaces that the Electron bridge supports.
    getVersionNumber: (callback: (version: string) => void) => callback('0.47.12.0'),
    getBrowserosVersionNumber: (callback: (version: string) => void) => callback('0.47.12.0'),
    getPref: (name: string, callback: (value: unknown) => void) => ipcRenderer.invoke('agent:pref-get', name).then(callback),
    setPref: (name: string, value: unknown, ...rest: unknown[]) => {
      const callback = typeof rest.at(-1) === 'function' ? rest.at(-1) as (value: boolean) => void : undefined
      return ipcRenderer.invoke('agent:pref-set', name, value).then((result) => callback?.(Boolean(result)))
    },
    logMetric: (eventName: string, ...rest: unknown[]) => {
      const properties = typeof rest[0] === 'object' ? rest[0] : undefined
      const callback = typeof rest.at(-1) === 'function' ? rest.at(-1) as () => void : undefined
      return ipcRenderer.invoke('agent:metric', eventName, properties).then(() => callback?.())
    },
    choosePath: (...rest: unknown[]) => {
      const callback = typeof rest.at(-1) === 'function' ? rest.at(-1) as (value: unknown) => void : undefined
      const options = typeof rest[0] === 'object' ? rest[0] : undefined
      return ipcRenderer.invoke('agent:choose-path', options).then((result) => callback?.(result))
    },
  },
  permissions: {
    contains: (request: unknown) => ipcRenderer.invoke('agent:permissions-contains', request),
    request: (request: unknown) => ipcRenderer.invoke('agent:permissions-request', request),
    remove: (request: unknown) => ipcRenderer.invoke('agent:permissions-remove', request),
    onAdded: eventChannel('agent:permission-added'),
    onRemoved: eventChannel('agent:permission-removed'),
  },
  alarms: {
    create: (name: string, info: unknown) => ipcRenderer.invoke('agent:alarm-create', name, info),
    clear: (name: string) => ipcRenderer.invoke('agent:alarm-clear', name),
    get: (name: string) => ipcRenderer.invoke('agent:alarm-get', name),
    getAll: () => ipcRenderer.invoke('agent:alarm-get-all'),
    clearAll: () => ipcRenderer.invoke('agent:alarm-clear-all'),
    onAlarm: eventChannel('agent:alarm'),
  },
  topSites: { get: () => ipcRenderer.invoke('agent:top-sites') },
  scripting: {
    executeScriptSerialized: (details: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:scripting-execute-script', details),
    executeScript: (details: Record<string, unknown> = {}) => {
      const func = details.func
      return ipcRenderer.invoke('agent:scripting-execute-script', {
        ...details,
        ...(typeof func === 'function' ? { func: func.toString() } : {}),
      })
    },
    insertCSS: (details: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:scripting-insert-css', details),
    removeCSS: (details: { target?: { tabId?: number }; cssOrigin?: string; key?: string } = {}) => ipcRenderer.invoke('agent:scripting-remove-css', details.target?.tabId, details.key),
  },
  webNavigation: {
    onCompleted: eventChannel('agent:web-navigation-completed'),
    onCommitted: eventChannel('agent:web-navigation-committed'),
  },
  tabGroups: {
    query: (queryInfo: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:tabgroups-query', queryInfo),
    get: (groupId: number) => ipcRenderer.invoke('agent:tabgroups-get', groupId),
    update: (groupId: number, updateProperties: Record<string, unknown> = {}) => ipcRenderer.invoke('agent:tabgroups-update', groupId, updateProperties),
  },
  bookmarks: {
    search: (query?: unknown) => ipcRenderer.invoke('agent:bookmark-search', query),
    get: (id: string) => ipcRenderer.invoke('agent:bookmark-get', id),
    getTree: () => ipcRenderer.invoke('agent:bookmark-tree'),
    create: (details: unknown) => ipcRenderer.invoke('agent:bookmark-create', details),
    remove: (id: string) => ipcRenderer.invoke('agent:bookmark-remove', id),
    update: (id: string, changes: unknown) => ipcRenderer.invoke('agent:bookmark-update', id, changes),
  },
  history: {
    search: (query?: unknown) => ipcRenderer.invoke('agent:history-search', query),
    addUrl: (details: unknown) => ipcRenderer.invoke('agent:history-add-url', details),
    deleteAll: () => ipcRenderer.invoke('agent:history-delete-all'),
    deleteUrl: (details: { url: string }) => ipcRenderer.invoke('agent:history-delete-url', details.url),
    deleteRange: (details: unknown) => ipcRenderer.invoke('agent:history-delete-range', details),
    getVisits: (details: { url: string }) => ipcRenderer.invoke('agent:history-visits', details.url),
  },
  downloads: {
    download: (options: unknown) => ipcRenderer.invoke('agent:download', options),
    search: (query?: unknown) => ipcRenderer.invoke('agent:downloads-search', query),
    open: (id: number) => ipcRenderer.invoke('agent:downloads-open', id),
    show: (id: number) => ipcRenderer.invoke('agent:downloads-show', id),
    erase: (query?: { id?: number }) => ipcRenderer.invoke('agent:downloads-erase', query?.id),
    cancel: (id: number) => ipcRenderer.invoke('agent:downloads-cancel', id),
    pause: (id: number) => ipcRenderer.invoke('agent:downloads-pause', id),
    resume: (id: number) => ipcRenderer.invoke('agent:downloads-resume', id),
    removeFile: (id: number) => ipcRenderer.invoke('agent:downloads-remove-file', id),
    onChanged: eventChannel('agent:download-changed'),
  },
}

contextBridge.exposeInMainWorld('__requestBrowserChrome', chromeShim)
contextBridge.executeInMainWorld({
  func: (api: typeof chromeShim) => {
    const root = globalThis as typeof globalThis & { chrome?: Record<string, unknown>; browser?: Record<string, unknown> }
    const chromeObject = root.chrome ?? {}
    Object.assign(chromeObject, api)
    const runtime = (chromeObject.runtime ?? api.runtime) as Record<string, unknown>
    Object.assign(runtime, api.runtime)
    if (!runtime.id) Object.defineProperty(runtime, 'id', { value: 'request-browser-electron', configurable: true })
    chromeObject.runtime = runtime
    const scripting = chromeObject.scripting as { executeScriptSerialized?: (details: Record<string, unknown>) => unknown } | undefined
    if (scripting?.executeScriptSerialized) {
      const executeScript = scripting.executeScriptSerialized
      chromeObject.scripting = {
        ...(chromeObject.scripting as Record<string, unknown>),
        executeScript: (details: Record<string, unknown>) => {
        const func = details.func
        return executeScript({
          ...details,
          ...(typeof func === 'function' ? { func: func.toString() } : {}),
        })
        },
      }
    }
    root.chrome = chromeObject
    root.browser = chromeObject
  },
  args: [chromeShim],
})
