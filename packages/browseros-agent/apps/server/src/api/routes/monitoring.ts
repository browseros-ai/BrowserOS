import { Hono } from 'hono'
import { getMonitoringService } from '../../monitoring/service'
import { isValidMonitoringRunId } from '../../monitoring/storage'

export function createMonitoringRoutes() {
  return new Hono()
    .get('/runs', async (c) => {
      const limitParam = c.req.query('limit')
      const parsedLimit = limitParam ? Number.parseInt(limitParam, 10) : 50
      const limit =
        Number.isFinite(parsedLimit) && parsedLimit > 0 ? parsedLimit : 50

      const runs = await getMonitoringService().listRuns(limit)
      return c.json({ runs })
    })
    .get('/runs/:id', async (c) => {
      const runId = c.req.param('id')
      if (!isValidMonitoringRunId(runId)) {
        return c.json({ error: 'Invalid monitoring run id' }, 400)
      }
      const envelope = await getMonitoringService().getRunEnvelope(runId)

      if (!envelope) {
        return c.json({ error: 'Monitoring run not found' }, 404)
      }

      return c.json({ run: envelope })
    })
    .post('/debug/runs', async (c) => {
      const body = await c.req.json<{
        agentId?: string
        sessionKey?: string
        originalPrompt?: string
        chatHistory?: Array<{ role?: 'user' | 'assistant'; content?: string }>
      }>()

      if (!body.agentId?.trim()) {
        return c.json({ error: 'agentId is required' }, 400)
      }
      if (!body.sessionKey?.trim()) {
        return c.json({ error: 'sessionKey is required' }, 400)
      }
      if (!body.originalPrompt?.trim()) {
        return c.json({ error: 'originalPrompt is required' }, 400)
      }

      const chatHistory = Array.isArray(body.chatHistory)
        ? body.chatHistory
            .filter(
              (turn): turn is { role: 'user' | 'assistant'; content: string } =>
                (turn.role === 'user' || turn.role === 'assistant') &&
                typeof turn.content === 'string',
            )
            .map((turn) => ({
              role: turn.role,
              content: turn.content,
            }))
        : []

      const session = await getMonitoringService().startSession({
        agentId: body.agentId.trim(),
        sessionKey: body.sessionKey.trim(),
        originalPrompt: body.originalPrompt.trim(),
        chatHistory,
        source: 'debug',
      })

      return c.json({ session }, 201)
    })
    .post('/debug/runs/:id/finalize', async (c) => {
      const runId = c.req.param('id')
      if (!isValidMonitoringRunId(runId)) {
        return c.json({ error: 'Invalid monitoring run id' }, 400)
      }
      const body = await c.req.json<{
        agentId?: string
        sessionKey?: string
        status?: 'completed' | 'failed' | 'aborted' | 'incomplete'
        finalAssistantMessage?: string
        error?: string
      }>()

      if (!body.agentId?.trim()) {
        return c.json({ error: 'agentId is required' }, 400)
      }
      if (!body.sessionKey?.trim()) {
        return c.json({ error: 'sessionKey is required' }, 400)
      }
      if (
        body.status !== 'completed' &&
        body.status !== 'failed' &&
        body.status !== 'aborted' &&
        body.status !== 'incomplete'
      ) {
        return c.json({ error: 'status is invalid' }, 400)
      }

      const envelope = await getMonitoringService().finalizeSession({
        monitoringSessionId: runId,
        agentId: body.agentId.trim(),
        sessionKey: body.sessionKey.trim(),
        status: body.status,
        finalAssistantMessage: body.finalAssistantMessage,
        error: body.error,
      })

      if (!envelope) {
        return c.json({ error: 'Monitoring run not found' }, 404)
      }

      return c.json({ run: envelope })
    })
}
