import { fileURLToPath, URL } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig(({ mode }) => {
  const isChromiumBuild = mode === 'chromium'

  return {
    base: isChromiumBuild ? './' : undefined,
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: {
        '@': fileURLToPath(new URL('./src', import.meta.url)),
      },
    },
    build: isChromiumBuild
      ? {
          outDir: 'dist/chromium',
          emptyOutDir: true,
          assetsDir: '.',
          cssCodeSplit: false,
          modulePreload: { polyfill: false },
          assetsInlineLimit: 0,
          rollupOptions: {
            output: {
              entryFileNames: 'app.js',
              chunkFileNames: 'app.js',
              inlineDynamicImports: true,
              assetFileNames: (assetInfo) => {
                const name = assetInfo.names[0] ?? ''
                if (name.endsWith('.css')) return 'app.css'
                throw new Error(
                  `Unexpected Chromium resource asset emitted: ${name}`,
                )
              },
            },
          },
        }
      : undefined,
  }
})
