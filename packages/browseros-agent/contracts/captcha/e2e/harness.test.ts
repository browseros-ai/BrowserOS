import { expect, test } from 'bun:test'
import { resolve } from 'node:path'
import { McpSession } from '../../claw-mcp/tests/mcp-client'

test('tests that an optional MCP call timeout actually bounds a slow response', async () => {
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      const rpc = (await request.json()) as { id?: number; method: string }
      if (rpc.method === 'notifications/initialized')
        return new Response(null, { status: 202 })
      if (rpc.method === 'tools/call') await Bun.sleep(60)
      return Response.json({
        jsonrpc: '2.0',
        id: rpc.id,
        result: { content: [] },
      })
    },
  })
  try {
    const mcp = await McpSession.connect(`http://127.0.0.1:${server.port}`)
    await expect(mcp.callTool('slow', {}, 5)).rejects.toThrow()
    expect((await mcp.callTool('slow', {}, 500)).content).toEqual([])
  } finally {
    await server.stop(true)
  }
})

test('tests that server binary overrides win and Cargo target directories are honored', () => {
  const module = resolve(import.meta.dir, '../../claw-mcp/tests/rust-server.ts')
  const binary = (env: NodeJS.ProcessEnv) => {
    const result = Bun.spawnSync(
      [
        process.execPath,
        '-e',
        `import {RUST_BINARY} from ${JSON.stringify(module)}; console.log(RUST_BINARY)`,
      ],
      {
        env: {
          ...process.env,
          CLAW_SERVER_BINARY: undefined,
          CARGO_TARGET_DIR: undefined,
          ...env,
        },
      },
    )
    expect(result.exitCode).toBe(0)
    return result.stdout.toString().trim()
  }
  expect(
    binary({
      CLAW_SERVER_BINARY: '/tmp/prebuilt-claw',
      CARGO_TARGET_DIR: '/tmp/cargo-test',
    }),
  ).toBe('/tmp/prebuilt-claw')
  expect(binary({ CARGO_TARGET_DIR: '/tmp/cargo-test' })).toBe(
    '/tmp/cargo-test/debug/browseros-claw-server-rs',
  )
  expect(binary({})).toEndWith(
    '/packages/browseros-agent/target/debug/browseros-claw-server-rs',
  )
})
