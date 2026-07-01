/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Standalone Vite dev server for the newtab UI. Serves the same
 * React app WXT bundles into the extension, at a plain
 * `http://127.0.0.1:5173` URL so browser automation (agent-browser,
 * chrome-devtools MCP, or a human's browser) can navigate it
 * without needing the extension installed.
 *
 * Bypasses the WXT/extension shell entirely: anything that depends
 * on `chrome.*` APIs would throw here. The audit / task detail
 * pages this config primarily targets talk to claw-server via
 * `fetch(http://127.0.0.1:9200/...)`; the claw-server already sends
 * `cors: origin: *` so no dev-server proxy is needed.
 *
 * Runs alongside `wxt dev` / `wxt build` without disturbing them;
 * this file is purely additive.
 */

import path from 'node:path'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig, type Plugin } from 'vite'

/**
 * Tailwind v4 auto-detects content sources relative to Vite's `root`.
 * Standalone `root` here is `entrypoints/newtab/`, so without this
 * plugin Tailwind would only scan those four files and ship a
 * stylesheet with theme + base but no utility classes. Prepending an
 * `@source "../../";` directive at transform time (in-memory only)
 * points Tailwind at the whole `apps/claw-app/` tree without editing
 * the on-disk `styles.css` that WXT also consumes.
 *
 * `enforce: 'pre'` runs this ahead of `@tailwindcss/vite` so the
 * injected directive is present when Tailwind parses the CSS.
 */
function injectTailwindSource(): Plugin {
  const STYLES_TAIL = '/entrypoints/newtab/styles.css'
  const IMPORT_LINE = '@import "tailwindcss";'
  return {
    name: 'browseros-claw-app:inject-tailwind-source',
    enforce: 'pre',
    transform(code, id) {
      if (!id.replace(/\\/g, '/').endsWith(STYLES_TAIL)) return null
      if (!code.includes(IMPORT_LINE)) return null
      return code.replace(IMPORT_LINE, `${IMPORT_LINE}\n@source "../../";`)
    },
  }
}

export default defineConfig({
  root: path.resolve(__dirname, 'entrypoints/newtab'),
  resolve: {
    alias: {
      '@': path.resolve(__dirname),
    },
  },
  plugins: [injectTailwindSource(), react(), tailwindcss()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
})
