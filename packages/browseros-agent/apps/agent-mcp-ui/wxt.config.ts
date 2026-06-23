import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'wxt'

// `entrypoints/newtab/` is WXT's conventional new-tab entrypoint. WXT
// auto-wires manifest.chrome_url_overrides.newtab to point at the
// generated newtab.html, so no hand-rolled override needed.
//
// `browserOS` is BrowserOS Chromium's permission gate for the
// new-tab override and the cockpit-adjacent surfaces.
export default defineConfig({
  outDir: 'dist',
  modules: ['@wxt-dev/module-react'],
  manifest: {
    name: 'BrowserOS Agents',
    permissions: [
      'browserOS',
      'storage',
      'tabs',
      'tabGroups',
      'sidePanel',
      'notifications',
      'webNavigation',
    ],
    host_permissions: ['http://127.0.0.1/*'],
    action: {
      default_title: 'BrowserOS Agents',
    },
  },
  vite: () => ({
    plugins: [tailwindcss()],
  }),
})
