/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { buildTestCommand, withTestEnv } from './__helpers__/run-test-group'

describe('withTestEnv', () => {
  it('defaults NODE_ENV to test when absent', () => {
    expect(withTestEnv({ PATH: '/usr/bin' }).NODE_ENV).toBe('test')
  })

  it('preserves an explicit NODE_ENV', () => {
    expect(withTestEnv({ NODE_ENV: 'production' }).NODE_ENV).toBe('production')
  })
})

describe('buildTestCommand', () => {
  it('preloads the test env bootstrap before running targets', () => {
    expect(buildTestCommand(['./tests/api'])).toEqual([
      process.execPath,
      '--env-file=.env.development',
      'test',
      '--preload=./tests/__helpers__/test-env.ts',
      './tests/api',
    ])
  })
})
