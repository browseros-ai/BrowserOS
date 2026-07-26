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
    name: 'BrowserClaw',
    key: 'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAyXbY2XVCs1/yJqGd53ei1rHdoUGIvZ8uq+x9YKmUc+jnb6NogIrq0USPeRNb6uzszio45GR8BW0O0pgbFKmhlhrCwgs9gEW8mufksE29E1g8Q2ug1sowzj38X6jmitO4I9cBbQMx7+gJZJS8pS5DZ+V7Bl8Uka2LWHMTP/Pf10YjbeNNCA0wj6kQkkTb8lg80r5Vm+gFqyo2xDFaxj8lN2kE73yFBjCt6B4ycntXvnnUTPX4IJqH+eQuwsFWPuqdYEwdvaaIOQ+lCxcYyZusX58zhxr0pkMxQjnEoJqAk6Av5O/JiNIOZYzbwUjm6aA+p9j9/6xzvmG+Lvp74Dk9pwIDAQAB',
    update_url: 'https://cdn.browseros.com/extensions/update-manifest.xml',
    // update_url: 'https://cdn.browseros.com/extensions/update-manifest.alpha.xml',
    // Mirrors apps/app/wxt.config.ts permissions (keep in sync — adding
    // permissions later re-prompts/disables existing installs on update),
    // followed by claw-specific extras.
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
      'notifications',
    ],
    // Recording is universal; the local server owns tab attribution.
    host_permissions: ['http://127.0.0.1/*', '<all_urls>'],
    action: {
      default_icon: {
        16: 'icon/16.png',
        32: 'icon/32.png',
        48: 'icon/48.png',
        128: 'icon/128.png',
      },
      default_title: 'BrowserClaw',
    },
  },
  vite: () => ({
    plugins: [tailwindcss()],
  }),
})
