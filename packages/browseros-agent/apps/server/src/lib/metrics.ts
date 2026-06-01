/**
 * @license
 * Copyright 2025 BrowserOS
 */
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { PostHog } from 'posthog-node'

import { INLINED_ENV } from '../env'

const POSTHOG_API_KEY = INLINED_ENV.POSTHOG_API_KEY
const EVENT_PREFIX = 'browseros.server.'

/**
 * The two events that previously fired per call and dwarfed every other
 * event in volume (~87% of total on the worst day). They get rolled up
 * into a single periodic `usage_rollup` event instead — see
 * `RollupBuffer` below.
 */
const AGGREGATED_EVENT_MCP_REQUEST = 'mcp.request'
const AGGREGATED_EVENT_TOOL_EXECUTED = 'tool_executed'

/**
 * Flush the rollup buffer every 30 minutes, wall-clock aligned so
 * dashboards comparing instances see aligned buckets. Not configurable
 * by design — one less knob to misconfigure in deployments. If a
 * future need surfaces, change the constant and ship a release.
 */
const ROLLUP_INTERVAL_MS = 30 * 60 * 1000

/**
 * Defensive cap on `tool_executions_by_tool` cardinality. Realistic
 * tool names are statically registered so this only matters if a
 * future code path passes dynamic / user-supplied names through.
 * Overflow names roll into `__other__`.
 */
const MAX_TOOL_NAME_KEYS = 200

/** PostHog property keys can't safely contain dots — they collide with
 *  nested-prop access syntax in HogQL. */
function sanitizeToolName(name: string): string {
  return name.replace(/\./g, '_').slice(0, 80)
}

export interface MetricsConfig {
  client_id?: string
  install_id?: string
  browseros_version?: string
  chromium_version?: string
  server_version?: string
  [key: string]: string | undefined
}

interface ToolExecCounters {
  total: number
  failed: number
  by_source: Record<string, number>
  by_tool: Record<string, number>
}

interface RollupBucket {
  period_start_ts: string
  mcp_requests: number
  tool_executions: ToolExecCounters
}

class RollupBuffer {
  private current: RollupBucket | null = null

  constructor(private readonly alignedNow: () => number = defaultAlignedNow) {}

  recordMcpRequest(): void {
    this.ensureCurrent().mcp_requests += 1
  }

  recordToolExecuted(props: {
    tool_name?: string
    source?: string
    success?: boolean
  }): void {
    const counters = this.ensureCurrent().tool_executions
    counters.total += 1
    if (props.success === false) counters.failed += 1
    const source = props.source ?? 'unknown'
    counters.by_source[source] = (counters.by_source[source] ?? 0) + 1
    if (props.tool_name) {
      const name = sanitizeToolName(props.tool_name)
      if (
        counters.by_tool[name] !== undefined ||
        Object.keys(counters.by_tool).length < MAX_TOOL_NAME_KEYS
      ) {
        counters.by_tool[name] = (counters.by_tool[name] ?? 0) + 1
      } else {
        counters.by_tool.__other__ = (counters.by_tool.__other__ ?? 0) + 1
      }
    }
  }

  /**
   * Return the current bucket and reset internal state. Returns `null`
   * when nothing was recorded since the last drain so callers can skip
   * the emit and avoid billing for an empty interval.
   */
  drain(
    periodEndTs: string,
  ): (RollupBucket & { period_end_ts: string }) | null {
    const bucket = this.current
    this.current = null
    if (!bucket) return null
    if (bucket.mcp_requests === 0 && bucket.tool_executions.total === 0) {
      return null
    }
    return { ...bucket, period_end_ts: periodEndTs }
  }

  private ensureCurrent(): RollupBucket {
    if (!this.current) {
      // Anchor period_start_ts to the aligned window boundary, not
      // to the moment the first event arrived. The flush always fires
      // on the boundary, so a lazy first-event timestamp would make
      // interval_seconds, period_start_ts, and period_end_ts disagree
      // about the window — and rate analyses computed as
      // `count / interval_seconds` would be wrong.
      this.current = {
        period_start_ts: new Date(this.alignedNow()).toISOString(),
        mcp_requests: 0,
        tool_executions: {
          total: 0,
          failed: 0,
          by_source: {},
          by_tool: {},
        },
      }
    }
    return this.current
  }
}

function defaultAlignedNow(): number {
  return Math.floor(Date.now() / ROLLUP_INTERVAL_MS) * ROLLUP_INTERVAL_MS
}

class MetricsService {
  private client: PostHog | null = null
  private config: MetricsConfig | null = null
  private rollup = new RollupBuffer()
  private flushTimer: ReturnType<typeof setTimeout> | null = null

  initialize(config: MetricsConfig): void {
    this.config = { ...this.config, ...config }

    if (!this.client && POSTHOG_API_KEY) {
      this.client = new PostHog(POSTHOG_API_KEY, {
        host: EXTERNAL_URLS.POSTHOG_DEFAULT,
      })
    }

    this.scheduleNextFlush()
  }

  isEnabled(): boolean {
    return this.client !== null
  }

  getClientId(): string | null {
    return this.config?.client_id ?? null
  }

  log(eventName: string, properties: Record<string, unknown> = {}): void {
    if (!this.client || !this.config) {
      return
    }

    // The two highest-volume events get aggregated. Every other event
    // captures per call as it always has.
    if (eventName === AGGREGATED_EVENT_MCP_REQUEST) {
      this.rollup.recordMcpRequest()
      return
    }
    if (eventName === AGGREGATED_EVENT_TOOL_EXECUTED) {
      this.rollup.recordToolExecuted({
        tool_name: properties.tool_name as string | undefined,
        source: properties.source as string | undefined,
        success: properties.success as boolean | undefined,
      })
      return
    }

    this.captureNow(eventName, properties)
  }

  async shutdown(): Promise<void> {
    if (this.flushTimer) {
      clearTimeout(this.flushTimer)
      this.flushTimer = null
    }
    // Drain whatever's buffered into the PostHog SDK before asking it
    // to flush its own send queue.
    this.flushRollup()
    if (this.client) {
      await this.client.shutdown()
      this.client = null
    }
  }

  private scheduleNextFlush(): void {
    if (this.flushTimer || !this.client) return
    const now = Date.now()
    // floor + 1 (not ceil) so a tick that lands exactly on a boundary
    // schedules to the *next* boundary instead of producing delay = 0
    // and immediately re-entering the recursive schedule.
    const nextTs =
      (Math.floor(now / ROLLUP_INTERVAL_MS) + 1) * ROLLUP_INTERVAL_MS
    const delay = Math.max(nextTs - now, 0)
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null
      this.flushRollup()
      this.scheduleNextFlush()
    }, delay)
    // Don't keep the event loop alive just for telemetry.
    this.flushTimer.unref?.()
  }

  private flushRollup(): void {
    const bucket = this.rollup.drain(new Date().toISOString())
    if (!bucket) return
    this.captureNow('usage_rollup', {
      interval_seconds: ROLLUP_INTERVAL_MS / 1000,
      period_start_ts: bucket.period_start_ts,
      period_end_ts: bucket.period_end_ts,
      mcp_requests_count: bucket.mcp_requests,
      tool_executions_count: bucket.tool_executions.total,
      tool_executions_failed: bucket.tool_executions.failed,
      tool_executions_by_source: bucket.tool_executions.by_source,
      tool_executions_by_tool: bucket.tool_executions.by_tool,
    })
  }

  private captureNow(
    eventName: string,
    properties: Record<string, unknown>,
  ): void {
    if (!this.client || !this.config) return

    const {
      client_id,
      install_id,
      browseros_version,
      chromium_version,
      server_version,
      ...defaultProperties
    } = this.config

    // No identity ⇒ no event. The previous `'anonymous'` fallback let
    // unconfigured instances funnel everything into one
    // un-attributable bucket and inflate billing dramatically. Treat
    // "no identity" as a configuration error to be surfaced at boot,
    // not as a reason to emit useless events.
    const distinctId = client_id || install_id
    if (!distinctId) return

    this.client.capture({
      distinctId,
      event: EVENT_PREFIX + eventName,
      properties: {
        ...defaultProperties,
        ...properties,
        ...(client_id && { client_id }),
        ...(install_id && { install_id }),
        ...(browseros_version && { browseros_version }),
        ...(chromium_version && { chromium_version }),
        ...(server_version && { server_version }),
        $process_person_profile: false,
      },
    })
  }
}

export const metrics = new MetricsService()

// Re-exported for tests; do not depend on these in product code.
export const __internal__ = {
  ROLLUP_INTERVAL_MS,
  MAX_TOOL_NAME_KEYS,
  sanitizeToolName,
  RollupBuffer,
}
