import { describe, expect, it } from 'bun:test'
import {
  InvalidMonitoringRunIdError,
  isValidMonitoringRunId,
  MonitoringStorage,
} from '../src/monitoring/storage'

describe('MonitoringStorage run id validation', () => {
  it('accepts UUID monitoring run ids', () => {
    expect(isValidMonitoringRunId('123e4567-e89b-12d3-a456-426614174000')).toBe(
      true,
    )
  })

  it('rejects path traversal run ids', async () => {
    expect(isValidMonitoringRunId('../../secret')).toBe(false)

    const storage = new MonitoringStorage()
    await expect(storage.readContext('../../secret')).rejects.toBeInstanceOf(
      InvalidMonitoringRunIdError,
    )
  })
})
