/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * Circuit Breaker — prevents cascading failures across MCP connections.
 * Open after 3 consecutive errors, half-open after cooldown, closed on success.
 */

export type CircuitState = 'closed' | 'open' | 'half-open'

export interface CircuitBreakerOptions {
  failureThreshold?: number
  cooldownMs?: number
  name?: string
}

export class CircuitBreaker {
  private state: CircuitState = 'closed'
  private failures = 0
  private lastFailureTime = 0
  private timer?: ReturnType<typeof setTimeout>
  readonly name: string
  readonly failureThreshold: number
  readonly cooldownMs: number

  constructor(opts: CircuitBreakerOptions = {}) {
    this.name = opts.name ?? 'circuit'
    this.failureThreshold = opts.failureThreshold ?? 3
    this.cooldownMs = opts.cooldownMs ?? 10000
  }

  get currentState(): CircuitState {
    if (this.state === 'open') {
      const elapsed = Date.now() - this.lastFailureTime
      if (elapsed >= this.cooldownMs) {
        this.state = 'half-open'
        console.log(`[CircuitBreaker:${this.name}] Half-open — probing…`)
      }
    }
    return this.state
  }

  /** Execute fn under circuit-breaker protection */
  async exec<T>(fn: () => Promise<T>): Promise<T> {
    const state = this.currentState

    if (state === 'open') {
      const retryAfter = Math.ceil(
        (this.cooldownMs - (Date.now() - this.lastFailureTime)) / 1000,
      )
      throw new CircuitOpenError(
        `Circuit breaker OPEN for ${this.name}. Retry after ${retryAfter}s.`,
        retryAfter,
      )
    }

    try {
      const result = await fn()
      this.onSuccess()
      return result
    } catch (err) {
      this.onFailure()
      throw err
    }
  }

  private onSuccess(): void {
    if (this.state === 'half-open') {
      console.log(`[CircuitBreaker:${this.name}] Closed — recovery confirmed.`)
    }
    this.state = 'closed'
    this.failures = 0
    if (this.timer) {
      clearTimeout(this.timer)
      this.timer = undefined
    }
  }

  private onFailure(): void {
    this.failures++
    this.lastFailureTime = Date.now()

    if (this.failures >= this.failureThreshold) {
      this.state = 'open'
      console.warn(
        `[CircuitBreaker:${this.name}] OPEN after ${this.failures} failures. Cooldown ${this.cooldownMs}ms.`,
      )
      this.timer = setTimeout(() => {
        console.log(
          `[CircuitBreaker:${this.name}] Cooldown expired — half-open.`,
        )
        this.state = 'half-open'
      }, this.cooldownMs)
    }
  }
}

export class CircuitOpenError extends Error {
  readonly retryAfterSeconds: number
  constructor(message: string, retryAfterSeconds: number) {
    super(message)
    this.name = 'CircuitOpenError'
    this.retryAfterSeconds = retryAfterSeconds
  }
}
