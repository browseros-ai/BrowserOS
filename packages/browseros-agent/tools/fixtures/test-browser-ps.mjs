#!/usr/bin/env node
/** Keep real process membership but hide the command keeper's marker to exercise failed ownership. */
import { spawnSync } from 'node:child_process'

const args = process.argv.slice(2)
const result = spawnSync('/bin/ps', args, {
  encoding: 'utf8',
  maxBuffer: 16 * 1024 * 1024,
})
const output = args.includes('-p')
  ? result.stdout
      .split('\n')
      .map((line) => line.replace(/ --command-owner .*/, ''))
      .join('\n')
  : result.stdout
process.stdout.write(output)
process.stderr.write(result.stderr)
process.exitCode = result.status ?? 1
