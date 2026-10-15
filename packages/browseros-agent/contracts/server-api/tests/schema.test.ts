/** Pins the compatibility surface independently of generated language models. */
import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import routes from './routes.json'
import {
  dereference,
  document,
  operation,
  pathItem,
  record,
  schemaFor,
} from './support'

test('pins every path ref, operation id, request ref and response dialect', () => {
  const paths = record(document().paths)
  expect(Object.keys(paths).sort()).toEqual(
    [...new Set(routes.map((route) => route.path))].sort(),
  )
  const ids = new Set<string>()
  let count = 0
  for (const route of routes) {
    expect(record(paths[route.path]).$ref).toBe(route.pathRef)
    const [op] = operation(route.path, route.method)
    expect(op.operationId).toBe(route.operationId)
    expect(op.requestBody).toEqual(route.requestBody ?? undefined)
    expect(op.responses).toEqual(route.responses)
    expect(ids.has(route.operationId)).toBe(false)
    ids.add(route.operationId)
  }
  for (const path of Object.keys(paths))
    count += Object.keys(pathItem(path)[0]).length
  expect(count).toBe(routes.length)
  expect(count).toBe(56)
})

test('all exported schemas and refs resolve; no version prefix is added', () => {
  for (const name of Object.keys(
    record(record(document().components).schemas),
  )) {
    expect(schemaFor(name)).toBeObject()
  }
  expect(
    Object.keys(record(document().paths)).some((path) =>
      path.startsWith('/api/'),
    ),
  ).toBe(false)
})

test('chat stream framing, replay status and root lifecycle aliases are explicit', () => {
  for (const [path, method] of [
    ['/chat', 'post'],
    ['/chat/{conversationId}/stream', 'get'],
  ]) {
    const [op, origin] = operation(path, method)
    const [response] = dereference(record(op.responses)['200'], origin)
    expect(
      record(
        record(record(response.headers)['x-vercel-ai-ui-message-stream'])
          .schema,
      ).enum,
    ).toEqual(['v1'])
    expect(record(response.content)['text/event-stream']).toBeDefined()
    expect(response.description).toContain('[DONE]')
  }
  expect(
    record(operation('/chat/{conversationId}/stream', 'get')[0].responses)[
      '204'
    ],
  ).toEqual({ description: 'No run record; no body.' })
  expect(schemaFor('HealthResponse').required).toEqual(['status'])
  expect(schemaFor('StatusResponse').properties).toHaveProperty('can_update')
  expect(record(operation('/mcp', 'delete')[0].responses)['404']).toBeDefined()
})

test('pins nullability, strict requests and provider projections', () => {
  expect(schemaFor('CreateBuiltinAgentRequest').additionalProperties).toBe(
    false,
  )
  expect(schemaFor('CreateCustomAgentRequest').required).toContain(
    'customConfig',
  )
  expect(schemaFor('UpsertProviderRequest').required).toEqual([
    'type',
    'name',
    'modelId',
    'contextWindow',
  ])
  expect(record(schemaFor('PublicProvider').properties)).toHaveProperty(
    'hasApiKey',
  )
  expect(record(schemaFor('PublicProvider').properties)).not.toHaveProperty(
    'apiKey',
  )
  expect(record(schemaFor('Provider').properties)).toHaveProperty('apiKey')
  expect(
    record(record(schemaFor('ScheduledJob').properties).providerId).anyOf,
  ).toContainEqual({ type: 'null' })
})

test('contract operations cover the actual TypeScript route registrations', () => {
  const sources: Array<[string, string[]]> = [
    ['health', ['/system/health', '/health']],
    ['status', ['/status']],
    ['diagnostics', ['/system/diagnostics']],
    ['shutdown', ['/system/shutdown', '/shutdown']],
    ['chat', ['/chat']],
    ['conversations', ['/conversations']],
    ['agents', ['/agents']],
    ['providers', ['/providers']],
    ['provider', ['/test-provider']],
    ['refine-prompt', ['/refine-prompt']],
    ['scheduled-jobs', ['/scheduled-jobs']],
    ['scheduled-job-runs', ['/scheduled-job-runs']],
    ['acpx-probe', ['/acpx/probe']],
    ['klavis', ['/klavis']],
    ['oauth', ['/oauth']],
    ['mcp', ['/mcp']],
    ['mcp-manager', ['/mcp-manager']],
  ]
  const registered: string[] = []
  for (const [file, mounts] of sources) {
    const source = readFileSync(
      join(
        import.meta.dir,
        '../../../apps/server/src/api/routes',
        `${file}.ts`,
      ),
      'utf8',
    )
    for (const match of source.matchAll(
      /\.(get|post|put|delete)\(\s*'(\/[^']*)'/g,
    )) {
      for (const mount of mounts) {
        const suffix = match[2] === '/' ? '' : match[2]
        registered.push(
          `${match[1]} ${mount}${suffix}`.replace(
            /:([A-Za-z][A-Za-z0-9]*)/g,
            '{$1}',
          ),
        )
      }
    }
  }
  // DELETE /mcp is explicitly documented as an unregistered plain-text 404.
  const contracted = routes
    .filter((route) => !(route.path === '/mcp' && route.method === 'delete'))
    .map((route) => `${route.method} ${route.path}`)
  expect(registered.sort()).toEqual(contracted.sort())
})
