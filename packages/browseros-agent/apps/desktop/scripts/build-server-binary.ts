import { mkdir } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'

const root = resolve(import.meta.dir, '../../..')
const output = join(root, 'apps/desktop/dist/request-browser-server.exe')

await mkdir(dirname(output), { recursive: true })

const result = Bun.spawnSync({
  cmd: [
    process.execPath,
    'build',
    'apps/server/src/index.ts',
    '--compile',
    '--outfile',
    output,
    '--target=bun-windows-x64-baseline',
  ],
  cwd: root,
  env: {
    ...process.env,
    NODE_ENV: 'production',
    BROWSEROS_CONFIG_URL: 'https://browseros.invalid/api/browseros-server/config',
    POSTHOG_API_KEY: 'phc_request_browser_disabled',
    SENTRY_DSN: 'https://request-browser.invalid/1',
  },
  stdout: 'inherit',
  stderr: 'inherit',
})

if (result.exitCode !== 0) process.exit(result.exitCode ?? 1)
