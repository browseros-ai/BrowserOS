/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { chmod, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export interface FakeSshResponse {
  stdout?: string
  stderr?: string
  exit?: number
}

export async function fakeSsh(
  response: FakeSshResponse = {},
  logPath?: string,
): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'fake-ssh-'))
  const path = join(dir, 'ssh')
  const body = `#!/usr/bin/env bash
set -u
echo "ARGS:$*" >> "${logPath ?? '/dev/null'}"
printf %b ${JSON.stringify(response.stdout ?? '')}
printf %b ${JSON.stringify(response.stderr ?? '')} >&2
exit ${response.exit ?? 0}
`
  await writeFile(path, body)
  await chmod(path, 0o755)
  return path
}
