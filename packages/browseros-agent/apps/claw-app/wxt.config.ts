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
    // TODO(Nikhil): Add this extension ID to Chromium's new-tab override allowlist.
    key: 'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvBDAaDRvv61NpBeLR8etBRw82lv9VJO3sz/mA26gDzWKtVuzW4DXCl8Zfj5oWmoXLTfv3aiTigUXo/LHOoGpSucEVroMmAc7cgu2KuQ1fZPpMvYa0npD/m4h89360q8Oz0oKKaZGS905IJ04M2IkF4CuU3YEHFJBWb+cUyK9H8YVugelYbPD0IVs63T1SkGbh/t/Tfb2DpkinduSO8+x26sKydm30SRt+iZ2+7Nolcdum3LExInUiX2Pgb65Jb+mVw8NqyTVJyCEp8uq0cSHomWFQirSJ80tsDhISp4btwaRKHrXqovQx9XHQv4hCd+3LuB830eUEVMUNuCO+OyPxQIDAQAB',
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
