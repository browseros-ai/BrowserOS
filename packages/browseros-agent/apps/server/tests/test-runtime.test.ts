/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { rmSync } from 'node:fs'
import {
  createTestRuntimePlan,
  type TestRuntimePlan,
} from './__helpers__/test-runtime'

const originalHeadless = process.env.BROWSEROS_TEST_HEADLESS
const runtimePlans: TestRuntimePlan[] = []

afterEach(() => {
  if (originalHeadless === undefined) {
    delete process.env.BROWSEROS_TEST_HEADLESS
  } else {
    process.env.BROWSEROS_TEST_HEADLESS = originalHeadless
  }

  for (const plan of runtimePlans) {
    rmSync(plan.userDataDir, { recursive: true, force: true })
  }
  runtimePlans.length = 0
})

async function resolveHeadless(value: string | undefined): Promise<boolean> {
  if (value === undefined) {
    delete process.env.BROWSEROS_TEST_HEADLESS
  } else {
    process.env.BROWSEROS_TEST_HEADLESS = value
  }

  const plan = await createTestRuntimePlan()
  runtimePlans.push(plan)
  return plan.headless
}

describe('createTestRuntimePlan headless mode', () => {
  it('defaults to headless when the flag is absent', async () => {
    expect(await resolveHeadless(undefined)).toBe(true)
  })

  it('uses headless mode when the flag is true', async () => {
    expect(await resolveHeadless('true')).toBe(true)
  })

  it('uses headed mode when the flag is false', async () => {
    expect(await resolveHeadless('false')).toBe(false)
  })
})
