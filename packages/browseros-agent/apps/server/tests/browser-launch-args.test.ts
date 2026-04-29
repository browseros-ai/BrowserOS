/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { killBrowser, spawnBrowser } from './__helpers__/browser'

async function waitForFile(path: string): Promise<void> {
  for (let attempt = 0; attempt < 50; attempt++) {
    if (existsSync(path)) {
      return
    }
    await new Promise((resolve) => setTimeout(resolve, 10))
  }
  throw new Error(`timed out waiting for ${path}`)
}

describe('spawnBrowser', () => {
  afterEach(async () => {
    await killBrowser()
  })

  it('uses the dev dock icon for server test browsers', async () => {
    const tempDir = mkdtempSync(join(tmpdir(), 'browseros-launch-args-'))
    const argsPath = join(tempDir, 'args.txt')
    const binaryPath = join(tempDir, 'browseros-fake')
    await Bun.write(
      binaryPath,
      [
        '#!/bin/sh',
        'args_file="$BROWSEROS_TEST_ARGS_FILE.tmp"',
        ': > "$args_file"',
        'for arg in "$@"; do',
        '  printf "%s\\n" "$arg" >> "$args_file"',
        'done',
        'mv "$args_file" "$BROWSEROS_TEST_ARGS_FILE"',
        'sleep 60',
      ].join('\n'),
    )
    chmodSync(binaryPath, 0o755)

    const cdpServer = createServer((_request, response) => {
      response.writeHead(200, { 'content-type': 'application/json' })
      response.end('{"Browser":"BrowserOS"}')
    })
    await new Promise<void>((resolve) => cdpServer.listen(0, resolve))
    const address = cdpServer.address()
    if (!address || typeof address === 'string') {
      throw new Error('failed to allocate CDP test port')
    }

    const originalArgsFile = process.env.BROWSEROS_TEST_ARGS_FILE
    process.env.BROWSEROS_TEST_ARGS_FILE = argsPath
    try {
      await spawnBrowser({
        cdpPort: address.port,
        serverPort: address.port + 1,
        extensionPort: address.port + 2,
        binaryPath,
        userDataDir: mkdtempSync(join(tmpdir(), 'browseros-test-')),
        headless: false,
        extraArgs: [],
      })

      await waitForFile(argsPath)
      const args = readFileSync(argsPath, 'utf8').split('\n')
      expect(args).toContain('--browseros-dock-icon=dev')
    } finally {
      if (originalArgsFile === undefined) {
        delete process.env.BROWSEROS_TEST_ARGS_FILE
      } else {
        process.env.BROWSEROS_TEST_ARGS_FILE = originalArgsFile
      }
      await killBrowser()
      await new Promise<void>((resolve, reject) => {
        cdpServer.close((error) => (error ? reject(error) : resolve()))
      })
      rmSync(tempDir, { recursive: true, force: true })
    }
  })
})
