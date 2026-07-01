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
import { defineConfig } from 'vite'

export default defineConfig({
  root: path.resolve(__dirname, 'entrypoints/newtab'),
  resolve: {
    alias: {
      '@': path.resolve(__dirname),
    },
  },
  plugins: [react(), tailwindcss()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
})
