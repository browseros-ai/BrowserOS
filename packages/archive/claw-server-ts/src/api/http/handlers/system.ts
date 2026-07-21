import type { SystemInfo } from '@browseros/claw-api'
import { RECORDING_INGEST_MAX_BYTES } from '@browseros/shared/constants/limits'
import { CLAW_API_PORT_DEFAULT } from '@browseros/shared/constants/ports'
import type { Handler } from 'hono'
import { getLocalServerUrl } from '../../../local-server-url'
import { VERSION } from '../../../version'
import type { RequestContextEnv } from '../request-context'

export interface SystemHandlerDependencies {
  getSystemInfo(): SystemInfo
  onShutdown?: () => void
}

export interface SystemHandlers {
  health: Handler<RequestContextEnv>
  shutdown: Handler<RequestContextEnv>
  info: Handler<RequestContextEnv>
}

export function createSystemHandlers(
  dependencies: SystemHandlerDependencies,
): SystemHandlers {
  return {
    health: (c) => c.json({ status: 'ok' as const }),
    shutdown: (c) => {
      setImmediate(() => dependencies.onShutdown?.())
      return c.json({ status: 'ok' as const })
    },
    info: (c) => c.json(dependencies.getSystemInfo()),
  }
}

export function createProductionSystemHandlers(
  onShutdown?: () => void,
): SystemHandlers {
  return createSystemHandlers({
    onShutdown,
    getSystemInfo: () => ({
      product: 'BrowserClaw',
      version: VERSION,
      url:
        getLocalServerUrl() ??
        `http://127.0.0.1:${CLAW_API_PORT_DEFAULT.toString()}`,
      capabilities: {
        recordingIngestVersion: 2,
        recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES,
      },
    }),
  })
}
