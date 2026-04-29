import { defineWebExtConfig } from 'wxt'

// biome-ignore lint/style/noProcessEnv: config file needs env access
const env = process.env

const chromiumArgs = [
  '--use-mock-keychain',
  '--show-component-extension-options',
  '--disable-browseros-server',
  '--disable-browseros-extensions',
  '--browseros-dock-icon=dev',
]

if (env.BROWSEROS_CDP_PORT) {
  // TODO: replace with --browseros-cdp-port once we fix the browseros bug
  chromiumArgs.push(`--remote-debugging-port=${env.BROWSEROS_CDP_PORT}`)
  // chromiumArgs.push(`--browseros-cdp-port =${env.BROWSEROS_CDP_PORT}`)
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
  chromiumProfile: env.BROWSEROS_USER_DATA_DIR || '/tmp/browseros-dev',
  keepProfileChanges: true,
  startUrls: ['chrome://newtab'],
})
