import { type DiagnosticsSnapshot, parseDiagnostics } from './contract'

export interface DiagnosticsResult {
  snapshot: DiagnosticsSnapshot
  cached: boolean
}

/** Bounds each dependency independently; a broken source cannot stall the report. */
export async function within<T>(
  task: () => Promise<T>,
  milliseconds: number,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      Promise.resolve().then(task),
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error('Diagnostics timed out')),
          milliseconds,
        )
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

/** One worker owns the cache and in-flight sample for all pages and external requests. */
export function createDiagnosticsCollector(deps: {
  read: () => Promise<DiagnosticsSnapshot>
  load: () => Promise<unknown>
  save: (snapshot: DiagnosticsSnapshot) => Promise<void>
}) {
  let pending: Promise<DiagnosticsResult> | undefined
  let latest: DiagnosticsSnapshot | null = null
  async function refresh(maxAgeMs: number): Promise<DiagnosticsResult> {
    const stored = await within(deps.load, 200).catch(() => null)
    latest ??= parseDiagnostics(stored)
    if (latest && maxAgeMs > 0) {
      const age = Date.now() - Date.parse(latest.collectedAt)
      if (age >= 0 && age < maxAgeMs) return { snapshot: latest, cached: true }
    }
    try {
      const snapshot = await within(deps.read, 2300)
      latest = snapshot
      await within(() => deps.save(snapshot), 200).catch(() => {})
      return { snapshot, cached: false }
    } catch (error) {
      if (latest) return { snapshot: latest, cached: true }
      throw error
    }
  }
  return {
    get(maxAgeMs: number): Promise<DiagnosticsResult> {
      pending ??= refresh(maxAgeMs).finally(() => {
        pending = undefined
      })
      return pending
    },
  }
}
