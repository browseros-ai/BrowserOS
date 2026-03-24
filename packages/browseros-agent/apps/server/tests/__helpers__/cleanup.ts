#!/usr/bin/env bun

import { existsSync } from 'node:fs'
import { readdir, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const ports = [
  Number(process.env.CDP_PORT ?? '9005'),
  Number(process.env.SERVER_PORT ?? '9105'),
  Number(process.env.EXTENSION_PORT ?? '9305'),
].filter((port) => Number.isFinite(port))

function parsePidsFromNetstat(output: string, targetPort: number): number[] {
  const pids = new Set<number>()
  const needle = `:${targetPort}`

  for (const rawLine of output.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || !line.includes(needle)) continue

    // Windows line example:
    // TCP    127.0.0.1:9105    0.0.0.0:0    LISTENING    12345
    // Unix line example:
    // tcp        0      0 127.0.0.1:9105      0.0.0.0:*       LISTEN      12345
    const parts = line.split(/\s+/)
    const pidStr = parts[parts.length - 1]
    const pid = Number(pidStr)
    if (Number.isFinite(pid) && pid > 0) {
      pids.add(pid)
    }
  }

  return [...pids]
}

async function killPortListeners(port: number): Promise<void> {
  const netstat = Bun.spawnSync({
    cmd: ['netstat', '-ano'],
    stdout: 'pipe',
    stderr: 'pipe',
  })

  const stdout = netstat.stdout.toString()
  if (!stdout) return

  const pids = parsePidsFromNetstat(stdout, port)
  for (const pid of pids) {
    try {
      process.kill(pid, 'SIGKILL')
      console.log(`  Killed process on port ${port} (PID: ${pid})`)
    } catch {
      // ignore failures (already exited or permission issue)
    }
  }
}

async function cleanupTempDirs(): Promise<void> {
  const tempRoot = process.env.TMPDIR || process.env.TMP || tmpdir()
  if (!existsSync(tempRoot)) return

  const entries = await readdir(tempRoot, { withFileTypes: true })
  const candidates = entries.filter(
    (entry) => entry.isDirectory() && entry.name.startsWith('browseros-test-'),
  )

  if (candidates.length === 0) return
  console.log(`  Removing ${candidates.length} orphaned temp directories`)

  await Promise.allSettled(
    candidates.map((entry) =>
      rm(join(tempRoot, entry.name), {
        recursive: true,
        force: true,
      }),
    ),
  )
}

async function main() {
  console.log('Cleaning up BrowserOS test resources...')
  for (const port of ports) {
    await killPortListeners(port)
  }
  await cleanupTempDirs()
  console.log('Cleanup complete')
}

await main()
