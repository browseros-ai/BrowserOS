import { metrics } from '../../../../lib/metrics'
import type { BrowserToolObserver } from '../browser-tool-dispatch'

/** Records operational metrics without changing the tool result. */
export const observeToolTelemetry: BrowserToolObserver = ({
  call,
  result,
  error,
  durationMs,
}) => {
  metrics.log('tool_executed', {
    tool_name: call.tool.name,
    source: call.source,
    duration_ms: durationMs,
    success: error === undefined && !result?.isError,
    ...(error !== undefined && { error_message: errorText(error) }),
  })
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
