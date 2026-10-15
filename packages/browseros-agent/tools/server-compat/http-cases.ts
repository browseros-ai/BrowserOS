/** Fixed HTTP request corpus; explicit statuses pin TS quirks as well as successful CRUD. */
export interface RequestCase {
  name: string
  path: string
  method?: string
  body?: unknown
  raw?: string
  origin?: string | null
  expected: number
  template?: string
}
const conversationId = '33333333-3333-4333-8333-333333333333'
const missingId = '44444444-4444-4444-8444-444444444444'
const provider = {
  type: 'openai-compatible',
  name: 'Compat fixture',
  modelId: 'plain-text',
  contextWindow: 4096,
  apiKey: 'synthetic-secret',
  createdAt: 1_700_000_000_000,
}
const job = {
  name: 'Compat fixture',
  query: 'Never scheduled',
  scheduleType: 'daily',
  scheduleTime: '09:00',
  enabled: false,
  providerId: 'compat-provider',
  createdAt: 1_700_000_000_000,
}

export const cases: RequestCase[] = [
  {
    name: 'health',
    path: '/system/health',
    expected: 200,
    template: '/system/health',
  },
  {
    name: 'health alias',
    path: '/health',
    expected: 200,
    template: '/health',
  },
  { name: 'status', path: '/status', expected: 200, template: '/status' },
  { name: 'empty providers', path: '/providers', expected: 200 },
  {
    name: 'provider create',
    path: '/providers/compat-provider',
    method: 'PUT',
    body: provider,
    expected: 200,
    template: '/providers/{providerId}',
  },
  {
    name: 'provider read redacts secret',
    path: '/providers/compat-provider',
    expected: 200,
    template: '/providers/{providerId}',
  },
  {
    name: 'provider update',
    path: '/providers/compat-provider',
    method: 'PUT',
    body: { ...provider, name: 'Renamed fixture' },
    expected: 200,
  },
  {
    name: 'provider set default',
    path: '/providers/default',
    method: 'PUT',
    body: { providerId: 'compat-provider' },
    expected: 200,
  },
  {
    name: 'provider read default',
    path: '/providers/default',
    expected: 200,
  },
  {
    name: 'conversation import',
    path: `/conversations/${conversationId}`,
    method: 'PUT',
    body: {
      messages: [
        {
          id: 'fixture-user',
          role: 'user',
          parts: [{ type: 'text', text: 'Fixture' }],
        },
      ],
      lastMessagedAt: 1_700_000_000_000,
    },
    expected: 200,
    template: '/conversations/{conversationId}',
  },
  {
    name: 'conversation read',
    path: `/conversations/${conversationId}`,
    expected: 200,
  },
  {
    name: 'conversation idempotent import',
    path: `/conversations/${conversationId}`,
    method: 'PUT',
    body: { messages: [] },
    expected: 200,
  },
  { name: 'conversation list', path: '/conversations', expected: 200 },
  {
    name: 'job create',
    path: '/scheduled-jobs/compat-job',
    method: 'PUT',
    body: job,
    expected: 200,
    template: '/scheduled-jobs/{jobId}',
  },
  {
    name: 'job update',
    path: '/scheduled-jobs/compat-job',
    method: 'PUT',
    body: { ...job, name: 'Renamed job' },
    expected: 200,
  },
  { name: 'job read', path: '/scheduled-jobs/compat-job', expected: 200 },
  { name: 'job list', path: '/scheduled-jobs', expected: 200 },
  {
    name: 'run create',
    path: '/scheduled-job-runs/compat-run',
    method: 'PUT',
    body: {
      jobId: 'compat-job',
      status: 'completed',
      startedAt: 1_700_000_000_000,
      completedAt: 1_700_000_000_100,
      result: 'Fixture',
    },
    expected: 200,
    template: '/scheduled-job-runs/{runId}',
  },
  { name: 'run read', path: '/scheduled-job-runs/compat-run', expected: 200 },
  { name: 'run list', path: '/scheduled-job-runs', expected: 200 },
]

export const agentCases = (id: string): RequestCase[] => [
  { name: 'agent read', path: `/agents/${id}`, expected: 200 },
  {
    name: 'agent update',
    path: `/agents/${id}`,
    method: 'PUT',
    body: { name: 'Renamed agent' },
    expected: 200,
  },
  { name: 'agent list', path: '/agents', expected: 200 },
  {
    name: 'malformed JSON',
    path: '/providers/broken',
    method: 'PUT',
    raw: '{',
    expected: 500,
    template: '/providers/{providerId}',
  },
  {
    name: 'missing required fields',
    path: '/providers/broken',
    method: 'PUT',
    body: {},
    expected: 400,
    template: '/providers/{providerId}',
  },
  {
    name: 'bad UUID',
    path: '/conversations/not-a-uuid',
    expected: 400,
    template: '/conversations/{conversationId}',
  },
  { name: 'bad chat UUID', path: '/chat/not-a-uuid/state', expected: 400 },
  {
    name: 'missing conversation',
    path: `/conversations/${missingId}`,
    expected: 404,
  },
  { name: 'missing provider', path: '/providers/absent', expected: 404 },
  { name: 'missing agent', path: `/agents/${missingId}`, expected: 404 },
  { name: 'missing job', path: '/scheduled-jobs/absent', expected: 404 },
  {
    name: 'forbidden origin read',
    path: '/providers',
    origin: 'https://forbidden.invalid',
    expected: 403,
  },
  {
    name: 'forbidden origin write',
    path: '/providers/broken',
    method: 'PUT',
    body: provider,
    origin: 'https://forbidden.invalid',
    expected: 403,
  },
  { name: 'no origin read', path: '/providers', origin: null, expected: 200 },
  {
    name: 'no origin write',
    path: '/providers/broken',
    method: 'PUT',
    body: provider,
    origin: null,
    expected: 403,
  },
  {
    name: 'CORS preflight omits PUT',
    path: '/providers',
    method: 'OPTIONS',
    expected: 204,
  },
  {
    name: 'negative MCP delete',
    path: '/mcp',
    method: 'DELETE',
    expected: 404,
  },
  {
    name: 'agent delete',
    path: `/agents/${id}`,
    method: 'DELETE',
    expected: 200,
  },
  {
    name: 'run delete',
    path: '/scheduled-job-runs/compat-run',
    method: 'DELETE',
    expected: 200,
  },
  {
    name: 'job delete',
    path: '/scheduled-jobs/compat-job',
    method: 'DELETE',
    expected: 200,
  },
  {
    name: 'conversation delete',
    path: `/conversations/${conversationId}`,
    method: 'DELETE',
    expected: 200,
  },
  {
    name: 'provider delete',
    path: '/providers/compat-provider',
    method: 'DELETE',
    expected: 200,
  },
]
