/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { sanitizeEvent } from '@browseros/shared/sentry/sanitize'
import * as Sentry from '@sentry/bun'

import { INLINED_ENV } from '../env'
import { VERSION } from '../version'
import { createEventBudget } from './sentry-throttle'

const SENTRY_ENVIRONMENT = process.env.NODE_ENV || 'development'

const eventBudget = createEventBudget()

// Ensure to call this before importing any other modules!
Sentry.init({
  dsn: INLINED_ENV.SENTRY_DSN,
  // Adds request headers and IP for users, for more info visit:
  // https://docs.sentry.io/platforms/javascript/guides/bun/configuration/options/#sendDefaultPii
  sendDefaultPii: true,
  environment: SENTRY_ENVIRONMENT,
  release: VERSION,

  beforeSend(event) {
    // Group tool execution errors by tool name instead of generic "execute"
    const message = event.exception?.values?.[0]?.value ?? ''
    if (message.startsWith('Internal error in ')) {
      const toolName = message.match(/Internal error in (\S+):/)?.[1]
      if (toolName) {
        event.fingerprint = ['tool-execution', toolName]
      }
    }

    // Cap each issue so one looping error cannot drain the Sentry quota. Runs
    // after fingerprinting so the budget groups events the way Sentry will.
    const suppressed = eventBudget.admit(event)
    if (suppressed === null) return null
    if (suppressed > 0) {
      event.tags = { ...event.tags, throttled_suppressed: suppressed }
    }

    return sanitizeEvent(event)
  },
})

export { Sentry }
