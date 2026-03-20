import { sentryVitePlugin } from '@sentry/vite-plugin'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'wxt'
import { LEGACY_AGENT_EXTENSION_ID } from './lib/constants/legacyAgentExtensionId'
import { PRODUCT_WEB_HOST } from './lib/constants/productWebHost'

// biome-ignore lint/style/noProcessEnv: build config file needs env access
const env = process.env

// biome-ignore lint/style/noNonNullAssertion: required env var
const apiUrl = new URL(env.VITE_PUBLIC_BROWSEROS_API!)
const apiPattern = apiUrl.port
  ? `${apiUrl.hostname}:${apiUrl.port}`
  : apiUrl.hostname

// See https://wxt.dev/api/config.html
// Extension ID will be bflpfmnmnokmjhmgnolecpppdbdophmk
export default defineConfig({
  outDir: 'dist',
  modules: ['@wxt-dev/module-react'],
  manifest: {
    name: 'Assistant',
    key: 'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvBDAaDRvv61NpBeLR8etBRw82lv9VJO3sz/mA26gDzWKtVuzW4DXCl8Zfj5oWmoXLTfv3aiTigUXo/LHOoGpSucEVroMmAc7cgu2KuQ1fZPpMvYa0npD/m4h89360q8Oz0oKKaZGS905IJ04M2IkF4CuU3YEHFJBWb+cUyK9H8YVugelYbPD0IVs63T1SkGbh/t/Tfb2DpkinduSO8+x26sKydm30SRt+iZ2+7Nolcdum3LExInUiX2Pgb65Jb+mVw8NqyTVJyCEp8uq0cSHomWFQirSJ80tsDhISp4btwaRKHrXqovQx9XHQv4hCd+3LuB830eUEVMUNuCO+OyPxQIDAQAB',
    update_url: 'https://cdn.browseros.com/extensions/update-manifest.xml',
    // update_url: 'https://cdn.browseros.com/extensions/update-manifest.alpha.xml',
    externally_connectable: {
      matches: [`https://${apiPattern}/*`, `https://*.${apiPattern}/*`],
    },
    web_accessible_resources: [
      {
        resources: ['app.html'],
        matches: [
          `https://${PRODUCT_WEB_HOST}/*`,
          `https://*.${PRODUCT_WEB_HOST}/*`,
        ],
        extension_ids: [LEGACY_AGENT_EXTENSION_ID],
      },
    ],
    chrome_url_overrides: {
      newtab: 'app.html',
    },
    options_ui: {
      page: 'app.html#/settings',
      open_in_tab: true,
    },
    action: {
      default_icon: {
        16: 'icon/16.png',
        32: 'icon/32.png',
        48: 'icon/48.png',
        128: 'icon/128.png',
      },
      default_title: 'Ask BrowserOS',
    },
    permissions: [
      'topSites',
      'tabs',
      'tabGroups',
      'storage',
      'sidePanel',
      'browserOS',
      'alarms',
    ],
    host_permissions: [
      'http://127.0.0.1/*',
      'https://suggestqueries.google.com/*',
      'https://api.bing.com/*',
      'https://in.search.yahoo.com/*',
      'https://duckduckgo.com/*',
      'https://search.brave.com/*',
    ],
  },
  vite: () => ({
    build: {
      sourcemap: 'hidden',
    },
    plugins: [
      tailwindcss(),
      ...(env.SENTRY_AUTH_TOKEN
        ? [
            sentryVitePlugin({
              org: env.SENTRY_ORG,
              project: env.SENTRY_PROJECT,
              authToken: env.SENTRY_AUTH_TOKEN,
              sourcemaps: {
                // Bug with sentry & WXT - refer: https://github.com/wxt-dev/wxt/issues/1735
                // filesToDeleteAfterUpload: ['./dist/**/*.map'],
              },
            }),
          ]
        : []),
    ],
  }),
})
