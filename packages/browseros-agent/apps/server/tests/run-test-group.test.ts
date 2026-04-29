/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  buildTestCommand,
  getAtomicGroupTargets,
  listAllGroups,
  withTestEnv,
} from './__helpers__/run-test-group'

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

describe('test groups', () => {
  it('includes the lib tests in the group list', () => {
    expect(listAllGroups()).toContain('lib')
  })

  it('runs root and directory integration tests in the integration group', () => {
    expect(getAtomicGroupTargets('integration')).toEqual([
      './tests/integration',
      './tests/server.integration.test.ts',
    ])
  })

  it('does not duplicate group names', () => {
    const groups = listAllGroups()

    expect(new Set(groups).size).toBe(groups.length)
  })
})
