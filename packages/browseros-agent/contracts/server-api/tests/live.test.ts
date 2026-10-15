/**
 * Opt-in conformance against an actual TS or Rust server. Only read-only GETs
 * and uniquely named CRUD fixtures run; OAuth, connectors, agent config writes,
 * chat execution and shutdown are deliberately excluded from live probes.
 */
import { expect, test } from 'bun:test'
import { assertResponse, record } from './support'

// biome-ignore lint/suspicious/noUndeclaredEnvVars: Opt-in direct Bun test; no Turbo cache.
const base = process.env.SERVER_API_BASE_URL?.replace(/\/+$/, '')
const origin =
  // biome-ignore lint/suspicious/noUndeclaredEnvVars: Opt-in direct Bun test; no Turbo cache.
  process.env.SERVER_API_ORIGIN ??
  'chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk'

async function request(
  path: string,
  method = 'get',
  body?: unknown,
  template = path,
  inspect?: (value: unknown) => void,
): Promise<unknown> {
  const response = await fetch(`${base}${path}`, {
    method: method.toUpperCase(),
    headers: {
      Origin: origin,
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(20_000),
  })
  const value: unknown = await response.json()
  inspect?.(value)
  assertResponse(template, method, response.status, value)
  // A documented 403/500 body is still a failed CRUD probe. Otherwise a broken
  // origin or unavailable store could make the cycle look conformant.
  if (!response.ok)
    throw new Error(`${method} ${template}: HTTP ${response.status}`)
  return value
}

test.skipIf(!base)(
  'live non-destructive GET responses conform',
  async () => {
    for (const path of [
      '/system/health',
      '/health',
      '/status',
      '/system/diagnostics',
      '/providers',
      '/providers/default',
      '/agents',
      '/conversations',
      '/scheduled-jobs',
      '/scheduled-job-runs',
      '/klavis/servers',
      '/mcp-manager/tools',
      '/mcp-manager/agents',
      '/mcp',
    ]) {
      await request(path)
    }
    await request(
      '/oauth/qwen-code/status',
      'get',
      undefined,
      '/oauth/{provider}/status',
    )
  },
  120_000,
)

test.skipIf(!base)(
  'live provider, conversation, job/run and agent create/read/delete cycle conforms',
  async () => {
    const token = crypto.randomUUID()
    const providerId = `contract-${token}`
    const conversationId = crypto.randomUUID()
    const jobId = `contract-${token}`
    const runId = `contract-${token}`
    const resources: Array<{ path: string; template: string }> = []
    const put = async (path: string, template: string, body: unknown) => {
      // Register cleanup before awaiting a write: a response validation failure
      // must not leave the successfully inserted resource behind.
      resources.push({ path, template })
      return request(path, 'put', body, template)
    }
    const failures: unknown[] = []
    try {
      await put(`/providers/${providerId}`, '/providers/{providerId}', {
        type: 'openai',
        name: `Contract ${token}`,
        modelId: 'contract-fixture',
        contextWindow: 4096,
      })
      await request(
        `/providers/${providerId}`,
        'get',
        undefined,
        '/providers/{providerId}',
      )
      const imported = record(
        await put(
          `/conversations/${conversationId}`,
          '/conversations/{conversationId}',
          {
            messages: [
              {
                id: 'fixture',
                role: 'user',
                parts: [{ type: 'text', text: 'Contract fixture' }],
              },
            ],
          },
        ),
      )
      expect(imported.imported).toBe(true)
      await request(
        `/conversations/${conversationId}`,
        'get',
        undefined,
        '/conversations/{conversationId}',
      )
      const existing = record(
        await request(
          `/conversations/${conversationId}`,
          'put',
          { messages: [] },
          '/conversations/{conversationId}',
        ),
      )
      expect(existing).toEqual({ conversation: null, imported: false })
      await put(`/scheduled-jobs/${jobId}`, '/scheduled-jobs/{jobId}', {
        name: `Contract ${token}`,
        query: 'Contract fixture; never scheduled',
        scheduleType: 'daily',
        scheduleTime: '09:00',
        enabled: false,
        providerId,
      })
      await request(
        `/scheduled-jobs/${jobId}`,
        'get',
        undefined,
        '/scheduled-jobs/{jobId}',
      )
      await put(`/scheduled-job-runs/${runId}`, '/scheduled-job-runs/{runId}', {
        jobId,
        status: 'completed',
        startedAt: Date.now(),
        completedAt: Date.now(),
        result: 'Contract fixture',
      })
      await request(
        `/scheduled-job-runs/${runId}`,
        'get',
        undefined,
        '/scheduled-job-runs/{runId}',
      )
      const created = record(
        await request(
          '/agents',
          'post',
          {
            name: `Contract ${token}`,
            type: 'codex',
          },
          '/agents',
          (value) => {
            const id = record(record(value).agent).id
            if (typeof id === 'string')
              resources.push({
                path: `/agents/${id}`,
                template: '/agents/{agentId}',
              })
          },
        ),
      )
      const id = record(created.agent).id
      if (typeof id !== 'string') throw new Error('Created agent has no id')
      await request(`/agents/${id}`, 'get', undefined, '/agents/{agentId}')
      await request(
        `/agents/${id}`,
        'put',
        { name: `Contract ${token} renamed` },
        '/agents/{agentId}',
      )
    } catch (error) {
      failures.push(error)
    } finally {
      // Reverse order respects the job/provider FK and deletes runs before jobs.
      // Attempt every cleanup even if one delete fails, then surface all failures.
      for (const resource of resources.reverse()) {
        try {
          await request(resource.path, 'delete', undefined, resource.template)
        } catch (error) {
          failures.push(error)
        }
      }
    }
    if (failures.length)
      throw new AggregateError(
        failures,
        'Live contract cycle or cleanup failed',
      )
  },
  120_000,
)
