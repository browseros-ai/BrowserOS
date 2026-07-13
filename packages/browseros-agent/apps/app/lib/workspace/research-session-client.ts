export interface WorkspaceResearchSession {
  id: string
  goal: string
  status: string
  plan?: Array<{ id: string; status: string }>
}

const DEFAULT_PLAN = [
  {
    title: 'Understand the goal and plan authorized browser actions',
    toolCategory: 'analysis',
  },
  {
    title: 'Browse authorized sources and collect evidence',
    toolCategory: 'browser',
  },
  {
    title: 'Verify findings and save structured results',
    toolCategory: 'database',
  },
] as const

async function workspaceRequest<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(`${baseUrl}${path}`, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
  })
  const body = (await response.json().catch(() => ({}))) as T & {
    error?: string
  }
  if (!response.ok) throw new Error(body.error || `Workspace request failed (${response.status})`)
  return body
}

export async function createResearchSessionForGoal(
  baseUrl: string,
  input: { goal: string; conversationId: string },
): Promise<WorkspaceResearchSession> {
  const response = await workspaceRequest<{ session: WorkspaceResearchSession }>(
    baseUrl,
    '/workspace/sessions',
    {
      method: 'POST',
      body: JSON.stringify({
        goal: input.goal,
        conversationId: input.conversationId,
        status: 'running',
        plan: DEFAULT_PLAN,
      }),
    },
  )
  return response.session
}

export async function updateResearchSessionStatus(
  baseUrl: string,
  sessionId: string,
  status: 'running' | 'completed' | 'failed' | 'paused',
): Promise<void> {
  await workspaceRequest(baseUrl, `/workspace/sessions/${sessionId}`, {
    method: 'PATCH',
    body: JSON.stringify({ status }),
  })
}

export async function recordResearchActivity(
  baseUrl: string,
  sessionId: string,
  input: { title: string; detail?: string; kind?: 'activity' | 'error' | 'checkpoint' },
): Promise<void> {
  await workspaceRequest(baseUrl, `/workspace/sessions/${sessionId}/events`, {
    method: 'POST',
    body: JSON.stringify({
      kind: input.kind ?? 'activity',
      title: input.title,
      detail: input.detail,
    }),
  })
}

export async function generateResearchRecap(
  baseUrl: string,
  sessionId: string,
): Promise<void> {
  await workspaceRequest(baseUrl, `/workspace/sessions/${sessionId}/recap`, {
    method: 'POST',
  })
}

export async function generateResearchSuggestion(
  baseUrl: string,
  sessionId: string,
): Promise<void> {
  await workspaceRequest(baseUrl, `/workspace/sessions/${sessionId}/suggestion`, {
    method: 'POST',
  })
}

export async function updateResearchPlanStep(
  baseUrl: string,
  stepId: string,
  status: 'pending' | 'running' | 'completed' | 'blocked' | 'skipped' | 'failed',
): Promise<void> {
  await workspaceRequest(baseUrl, `/workspace/plan-steps/${stepId}`, {
    method: 'PATCH',
    body: JSON.stringify({ status }),
  })
}
