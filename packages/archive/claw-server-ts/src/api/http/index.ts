import { Hono } from 'hono'
import type { ConnectionHandlers } from './handlers/connections'
import type { PreviewHandlers } from './handlers/previews'
import type { RecordingHandlers } from './handlers/recordings'
import type { ReplayHandlers } from './handlers/replay'
import type { ScreenshotHandlers } from './handlers/screenshots'
import type { SessionHandlers } from './handlers/sessions'
import type { SettingsHandlers } from './handlers/settings'
import type { SystemHandlers } from './handlers/system'
import { recordingBodyLimit } from './middleware/recording-body-limit'
import type { RequestContextEnv } from './request-context'

export interface HttpHandlerSet {
  system: SystemHandlers
  settings: SettingsHandlers
  sessions: SessionHandlers
  recordings: RecordingHandlers
  replay: ReplayHandlers
  previews: PreviewHandlers
  screenshots: ScreenshotHandlers
  connections: ConnectionHandlers
}

/** Central method/path table for the BrowserClaw HTTP API. */
export function createHttpRoute(handlers: HttpHandlerSet) {
  const app = new Hono<RequestContextEnv>()

  app.get('/system/health', handlers.system.health)
  app.post('/system/shutdown', handlers.system.shutdown)
  app.get('/api/v1/system', handlers.system.info)
  app.get('/api/v1/settings/telemetry', handlers.settings.getTelemetry)
  app.put('/api/v1/settings/telemetry', handlers.settings.updateTelemetry)
  app.get('/api/v1/sessions', handlers.sessions.list)
  app.get('/api/v1/sessions/:sessionId', handlers.sessions.detail)
  app.post('/api/v1/sessions/:sessionId/cancel', handlers.sessions.cancel)
  app.get('/api/v1/sessions/:sessionId/recording', handlers.replay.metadata)
  app.get(
    '/api/v1/sessions/:sessionId/recording/events',
    handlers.replay.events,
  )
  app.post(
    '/api/v1/recordings/events',
    recordingBodyLimit(),
    handlers.recordings.ingest,
  )
  app.get('/api/v1/sessions/:sessionId/preview', handlers.previews.get)
  app.get('/api/v1/sessions/:sessionId/screenshots', handlers.screenshots.list)
  app.get(
    '/api/v1/sessions/:sessionId/screenshots/:screenshotId',
    handlers.screenshots.get,
  )
  app.get('/api/v1/connections', handlers.connections.list)
  app.put('/api/v1/connections/:harness', handlers.connections.connect)
  app.delete('/api/v1/connections/:harness', handlers.connections.disconnect)

  return app
}
