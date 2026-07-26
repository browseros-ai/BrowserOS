/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import pc from 'picocolors'

export const log = {
  header: (title: string) => console.log(`\n${pc.bold(title)}`),
  step: (msg: string) => console.log(pc.cyan(`  → ${msg}`)),
  success: (msg: string) => console.log(pc.green(`  ✓ ${msg}`)),
  warn: (msg: string) => console.log(pc.yellow(`  ! ${msg}`)),
  error: (msg: string) => console.log(pc.red(`  ✗ ${msg}`)),
  info: (msg: string) => console.log(pc.dim(`    ${msg}`)),
  done: (msg: string) => console.log(pc.green(`\n✓ ${msg}\n`)),
  fail: (msg: string) => console.log(pc.red(`\n✗ ${msg}\n`)),
}
