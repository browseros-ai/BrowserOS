import { sentryVitePlugin } from '@sentry/vite-plugin'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'wxt'

// The packaged Electron build owns the local runtime and does not publish an
// extension update feed or expose the app to the upstream BrowserOS domains.
// Provider/API compatibility remains in the application runtime where it is
// explicitly configured by the user.
// biome-ignore lint/style/noProcessEnv: build config file needs env access
const env = process.env

// See https://wxt.dev/api/config.html
// Extension ID will be bflpfmnmnokmjhmgnolecpppdbdophmk
export default defineConfig({
  outDir: 'dist',
  modules: ['@wxt-dev/module-react'],
  manifest: {
    name: 'Request Browser',
    description: 'Request Browser AI interface',
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
      default_title: 'Ask Request Browser',
    },
    permissions: [
      'topSites',
      'storage',
      'unlimitedStorage',
      'scripting',
      'tabs',
      'tabGroups',
      'sidePanel',
      'bookmarks',
      'history',
      'browserOS',
      'alarms',
      'webNavigation',
      'downloads',
    ],
    host_permissions: ['http://127.0.0.1/*'],
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
