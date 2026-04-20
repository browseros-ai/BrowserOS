import {
  appendFile,
  mkdir,
  readdir,
  readFile,
  stat,
  writeFile,
} from 'node:fs/promises'
import { join } from 'node:path'
import {
  getLazyMonitoringRunDir,
  getLazyMonitoringRunsDir,
} from '../lib/browseros-dir'
import type {
  MonitoringFinalization,
  MonitoringSessionContext,
  MonitoringToolCallRecord,
} from './types'

const CONTEXT_FILE_NAME = 'context.json'
const TOOL_CALLS_FILE_NAME = 'tool-calls.jsonl'
const FINALIZATION_FILE_NAME = 'finalization.json'
const AUDIT_ENVELOPE_FILE_NAME = 'audit-envelope.json'
const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i

export class InvalidMonitoringRunIdError extends Error {
  constructor(runId: string) {
    super(`Invalid monitoring run id: ${runId}`)
    this.name = 'InvalidMonitoringRunIdError'
  }
}

export function isValidMonitoringRunId(runId: string): boolean {
  return UUID_PATTERN.test(runId)
}

function assertValidMonitoringRunId(runId: string): void {
  if (!isValidMonitoringRunId(runId)) {
    throw new InvalidMonitoringRunIdError(runId)
  }
}

export class MonitoringStorage {
  async writeContext(context: MonitoringSessionContext): Promise<void> {
    await this.ensureRunDir(context.monitoringSessionId)
    await writeFile(
      this.getContextPath(context.monitoringSessionId),
      `${JSON.stringify(context, null, 2)}\n`,
    )
  }

  async appendToolCall(record: MonitoringToolCallRecord): Promise<void> {
    await this.ensureRunDir(record.monitoringSessionId)
    await appendFile(
      this.getToolCallsPath(record.monitoringSessionId),
      `${JSON.stringify(record)}\n`,
    )
  }

  async writeFinalization(finalization: MonitoringFinalization): Promise<void> {
    await this.ensureRunDir(finalization.monitoringSessionId)
    await writeFile(
      this.getFinalizationPath(finalization.monitoringSessionId),
      `${JSON.stringify(finalization, null, 2)}\n`,
    )
  }

  async writeAuditEnvelope(runId: string, envelope: unknown): Promise<void> {
    await this.ensureRunDir(runId)
    await writeFile(
      this.getAuditEnvelopePath(runId),
      `${JSON.stringify(envelope, null, 2)}\n`,
    )
  }

  async readContext(runId: string): Promise<MonitoringSessionContext | null> {
    return this.readJsonFile<MonitoringSessionContext>(
      this.getContextPath(runId),
    )
  }

  async readFinalization(
    runId: string,
  ): Promise<MonitoringFinalization | null> {
    return this.readJsonFile<MonitoringFinalization>(
      this.getFinalizationPath(runId),
    )
  }

  async readToolCalls(runId: string): Promise<MonitoringToolCallRecord[]> {
    try {
      const content = await readFile(this.getToolCallsPath(runId), 'utf8')
      return content
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean)
        .flatMap((line) => {
          try {
            return [JSON.parse(line) as MonitoringToolCallRecord]
          } catch {
            return []
          }
        })
    } catch {
      return []
    }
  }

  async listRunIds(): Promise<string[]> {
    try {
      const entries = await readdir(getLazyMonitoringRunsDir(), {
        withFileTypes: true,
      })
      const directories = entries.filter(
        (entry) => entry.isDirectory() && isValidMonitoringRunId(entry.name),
      )
      const runStats = await Promise.all(
        directories.map(async (entry) => ({
          runId: entry.name,
          mtimeMs: await this.getDirectoryMtimeMs(entry.name),
        })),
      )
      return runStats
        .sort((a, b) => b.mtimeMs - a.mtimeMs)
        .map((entry) => entry.runId)
    } catch {
      return []
    }
  }

  private async ensureRunDir(runId: string): Promise<void> {
    assertValidMonitoringRunId(runId)
    await mkdir(getLazyMonitoringRunsDir(), { recursive: true })
    await mkdir(getLazyMonitoringRunDir(runId), { recursive: true })
  }

  private async getDirectoryMtimeMs(runId: string): Promise<number> {
    try {
      const info = await stat(getLazyMonitoringRunDir(runId))
      return info.mtimeMs
    } catch {
      return 0
    }
  }

  private async readJsonFile<T>(path: string): Promise<T | null> {
    try {
      const content = await readFile(path, 'utf8')
      return JSON.parse(content) as T
    } catch {
      return null
    }
  }

  private getContextPath(runId: string): string {
    assertValidMonitoringRunId(runId)
    return join(getLazyMonitoringRunDir(runId), CONTEXT_FILE_NAME)
  }

  private getToolCallsPath(runId: string): string {
    assertValidMonitoringRunId(runId)
    return join(getLazyMonitoringRunDir(runId), TOOL_CALLS_FILE_NAME)
  }

  private getFinalizationPath(runId: string): string {
    assertValidMonitoringRunId(runId)
    return join(getLazyMonitoringRunDir(runId), FINALIZATION_FILE_NAME)
  }

  private getAuditEnvelopePath(runId: string): string {
    assertValidMonitoringRunId(runId)
    return join(getLazyMonitoringRunDir(runId), AUDIT_ENVELOPE_FILE_NAME)
  }
}
