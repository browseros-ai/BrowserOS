import { logger } from '../lib/logger'
import type { MonitoringToolEndInput, MonitoringToolStartInput } from './types'

export interface ToolExecutionObserver {
  onToolStart(input: MonitoringToolStartInput): Promise<void>
  onToolEnd(input: MonitoringToolEndInput): Promise<void>
}

export function swallowMonitoringError(
  operation: string,
  error: unknown,
  metadata: Record<string, unknown>,
): void {
  logger.warn(`Lazy monitoring ${operation} failed`, {
    ...metadata,
    error: error instanceof Error ? error.message : String(error),
  })
}
