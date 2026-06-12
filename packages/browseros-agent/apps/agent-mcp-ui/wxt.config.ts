import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'wxt'

// Mounts the React-only build pipeline (Vite + @vitejs/plugin-react)
// via @wxt-dev/module-react and reserves the cockpit at
// chrome_url_overrides.newtab so the extension takes over the
// new-tab page once installed.
export default defineConfig({
  outDir: 'dist',
  modules: ['@wxt-dev/module-react'],
  manifest: {
    name: 'BrowserOS Agents',
    // `browserOS` is BrowserOS Chromium's gate for the new-tab override
    // and other agent-cockpit-relevant surfaces. Without it the
    // `chrome_url_overrides.newtab` declaration silently no-ops and
    // BrowserOS keeps its default new-tab page.
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
    chrome_url_overrides: { newtab: 'app.html' },
    action: {
      default_title: 'BrowserOS Agents',
    },
  },
  vite: () => ({
    plugins: [tailwindcss()],
  }),
})
