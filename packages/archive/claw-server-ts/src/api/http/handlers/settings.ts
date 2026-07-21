import type { TelemetryState } from '@browseros/claw-api'
import type { Handler } from 'hono'
import {
  getTelemetryState,
  setTelemetryConsent,
} from '../../../services/analytics'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'

export interface SettingsHandlerDependencies {
  getTelemetry(): TelemetryState
  updateTelemetry(consent: boolean): TelemetryState
}

export interface SettingsHandlers {
  getTelemetry: Handler<RequestContextEnv>
  updateTelemetry: Handler<RequestContextEnv>
}

export function createSettingsHandlers(
  dependencies: SettingsHandlerDependencies,
): SettingsHandlers {
  return {
    getTelemetry: (c) => c.json(dependencies.getTelemetry()),
    updateTelemetry: async (c) => {
      const body = await readJson(c.req.raw)
      if (!isConsentBody(body)) {
        return apiError(c, 400, 'invalid_request', 'consent must be a boolean')
      }
      return c.json(dependencies.updateTelemetry(body.consent))
    },
  }
}

export const productionSettingsHandlers = createSettingsHandlers({
  getTelemetry: getTelemetryState,
  updateTelemetry: setTelemetryConsent,
})

async function readJson(request: Request): Promise<unknown> {
  try {
    return await request.json()
  } catch {
    return null
  }
}

function isConsentBody(value: unknown): value is { consent: boolean } {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { consent?: unknown }).consent === 'boolean'
  )
}
