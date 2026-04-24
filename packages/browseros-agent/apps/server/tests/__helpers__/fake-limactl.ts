/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { chmod, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export interface FakeLimactlResponse {
  stdout?: string
  stderr?: string
  exit?: number
}

export async function fakeLimactl(
  canned: Record<string, FakeLimactlResponse>,
  logPath?: string,
): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'fake-limactl-'))
  const path = join(dir, 'limactl')
  const limaHomeExpansion = '$' + '{LIMA_HOME-}'
  const cases = Object.entries(canned)
    .map(([command, response]) =>
      [
        `  ${JSON.stringify(command)})`,
        `    echo "ARGS:$*" >> "${logPath ?? '/dev/null'}"`,
        `    echo "LIMA_HOME:${limaHomeExpansion}" >> "${logPath ?? '/dev/null'}"`,
        `    printf %b ${JSON.stringify(response.stdout ?? '')}`,
        `    printf %b ${JSON.stringify(response.stderr ?? '')} >&2`,
        `    exit ${response.exit ?? 0}`,
        '    ;;',
      ].join('\n'),
    )
    .join('\n')
  const body = `#!/usr/bin/env bash
set -u
case "$1" in
${cases}
  *)
    echo "ARGS:$*" >> "${logPath ?? '/dev/null'}"
    echo "unexpected subcommand: $1" >&2
    exit 99
    ;;
esac
`
  await writeFile(path, body)
  await chmod(path, 0o755)
  return path
}
