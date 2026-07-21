import { describe, expect, test } from 'bun:test'
import type { AcpRuntimeTurnResultError } from 'acpx/runtime'
import {
  AcpxAgentNotFoundError,
  AcpxAuthRequiredError,
  AcpxError,
  AcpxTurnTimeoutError,
  fromRuntimeError,
} from '../../src/errors.ts'

describe('AcpxError', () => {
  test('sets code, retryable, and cause from options', () => {
    const cause = new Error('underlying')
    const err = new AcpxError('boom', {
      code: 'unknown',
      retryable: true,
      cause,
    })

    expect(err.message).toBe('boom')
    expect(err.code).toBe('unknown')
    expect(err.retryable).toBe(true)
    expect(err.cause).toBe(cause)
    expect(err.name).toBe('AcpxError')
    expect(err).toBeInstanceOf(Error)
  })

  test('defaults retryable to false', () => {
    const err = new AcpxError('boom', { code: 'x' })
    expect(err.retryable).toBe(false)
  })

  test('omits cause when not provided', () => {
    const err = new AcpxError('boom', { code: 'x' })
    expect(err.cause).toBeUndefined()
  })
})

describe('AcpxAgentNotFoundError', () => {
  test('uses fixed agent_not_found code and is not retryable', () => {
    const err = new AcpxAgentNotFoundError('claude not on PATH')

    expect(err.code).toBe('agent_not_found')
    expect(err.retryable).toBe(false)
    expect(err.name).toBe('AcpxAgentNotFoundError')
    expect(err).toBeInstanceOf(AcpxError)
  })

  test('appends doctor report when provided', () => {
    const err = new AcpxAgentNotFoundError('claude not on PATH', {
      doctor: 'agent registry: pi=npx pi-acp@^0.0.26',
    })

    expect(err.message).toContain('claude not on PATH')
    expect(err.message).toContain('agent registry')
  })

  test('threads cause through', () => {
    const cause = new Error('spawn failed')
    const err = new AcpxAgentNotFoundError('claude not on PATH', { cause })
    expect(err.cause).toBe(cause)
  })
})

describe('AcpxAuthRequiredError', () => {
  test('uses default actionable message when none provided', () => {
    const err = new AcpxAuthRequiredError()

    expect(err.message).toContain('ACPX_AUTH_')
    expect(err.code).toBe('auth_required')
    expect(err.retryable).toBe(false)
    expect(err.name).toBe('AcpxAuthRequiredError')
    expect(err).toBeInstanceOf(AcpxError)
  })

  test('respects custom message', () => {
    const err = new AcpxAuthRequiredError('login first')
    expect(err.message).toBe('login first')
  })
})

describe('AcpxTurnTimeoutError', () => {
  test('is retryable and uses turn_timeout code', () => {
    const err = new AcpxTurnTimeoutError('turn took too long')

    expect(err.retryable).toBe(true)
    expect(err.code).toBe('turn_timeout')
    expect(err.name).toBe('AcpxTurnTimeoutError')
    expect(err).toBeInstanceOf(AcpxError)
  })
})

describe('fromRuntimeError', () => {
  test('maps auth_required code to AcpxAuthRequiredError', () => {
    const src: AcpRuntimeTurnResultError = {
      message: 'unauthorized',
      code: 'auth_required',
    }
    const err = fromRuntimeError(src)

    expect(err).toBeInstanceOf(AcpxAuthRequiredError)
    expect(err.cause).toBe(src)
  })

  test('matches authentication hint via message even without code', () => {
    const src: AcpRuntimeTurnResultError = { message: 'Authentication failed' }
    expect(fromRuntimeError(src)).toBeInstanceOf(AcpxAuthRequiredError)
  })

  test('maps agent_not_found code to AcpxAgentNotFoundError', () => {
    const src: AcpRuntimeTurnResultError = {
      message: 'no such agent',
      code: 'agent_not_found',
    }
    expect(fromRuntimeError(src)).toBeInstanceOf(AcpxAgentNotFoundError)
  })

  test('maps ACP_BACKEND_MISSING runtime code to AcpxAgentNotFoundError', () => {
    const src: AcpRuntimeTurnResultError = {
      message: 'backend missing',
      code: 'ACP_BACKEND_MISSING',
    }
    expect(fromRuntimeError(src)).toBeInstanceOf(AcpxAgentNotFoundError)
  })

  test('maps "timed out" message to AcpxTurnTimeoutError', () => {
    const src: AcpRuntimeTurnResultError = { message: 'turn timed out' }
    expect(fromRuntimeError(src)).toBeInstanceOf(AcpxTurnTimeoutError)
  })

  test('falls through to plain AcpxError for unknown codes', () => {
    const src: AcpRuntimeTurnResultError = {
      message: 'something else',
      code: 'unrecognized',
    }
    const err = fromRuntimeError(src)

    expect(err).toBeInstanceOf(AcpxError)
    expect(err).not.toBeInstanceOf(AcpxAgentNotFoundError)
    expect(err).not.toBeInstanceOf(AcpxAuthRequiredError)
    expect(err).not.toBeInstanceOf(AcpxTurnTimeoutError)
    expect(err.code).toBe('unrecognized')
  })

  test('preserves retryable flag when falling through', () => {
    const src: AcpRuntimeTurnResultError = {
      message: 'rate limited',
      code: 'rate_limit',
      retryable: true,
    }
    const err = fromRuntimeError(src)

    expect(err.retryable).toBe(true)
    expect(err.code).toBe('rate_limit')
  })

  test('defaults code to "unknown" when runtime error omits it', () => {
    const src: AcpRuntimeTurnResultError = { message: 'mystery' }
    const err = fromRuntimeError(src)
    expect(err.code).toBe('unknown')
  })

  test('attaches the runtime error itself as cause', () => {
    const src: AcpRuntimeTurnResultError = { message: 'failed', code: 'x' }
    expect(fromRuntimeError(src).cause).toBe(src)
  })
})
