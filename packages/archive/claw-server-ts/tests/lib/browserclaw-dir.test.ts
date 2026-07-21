/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { env } from '../../src/env'
import {
  getClawServerDir,
  resolveClawServerPath,
} from '../../src/lib/browserclaw-dir'

const prior = {
  browserClawDirOverride: env.browserClawDirOverride,
  isDevelopment: env.isDevelopment,
}

afterEach(() => {
  env.browserClawDirOverride = prior.browserClawDirOverride
  env.isDevelopment = prior.isDevelopment
})

describe('browserclaw-dir', () => {
  test('uses the BrowserClaw production state dir by default', () => {
    env.browserClawDirOverride = undefined
    env.isDevelopment = false

    expect(getClawServerDir()).toBe(join(homedir(), '.browserclaw'))
  })

  test('uses the BrowserClaw development state dir in development', () => {
    env.browserClawDirOverride = undefined
    env.isDevelopment = true

    expect(getClawServerDir()).toBe(join(homedir(), '.browserclaw-dev'))
  })

  test('uses an override as the BrowserClaw root without adding a subdir', () => {
    env.browserClawDirOverride = '/tmp/browserclaw-root'

    expect(getClawServerDir()).toBe('/tmp/browserclaw-root')
    expect(resolveClawServerPath('agents', 'one.json')).toBe(
      '/tmp/browserclaw-root/agents/one.json',
    )
  })
})
