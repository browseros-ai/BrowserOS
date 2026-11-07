/**
 * Playwright: `client/webmcp.ts` `WebMCP`
 *
 * `frame.webmcp` (and `page.webmcp`, its main frame's): the tools the document registered with
 * WebMCP, read and called in the frame's utility world (neo-runtime `webmcp.rs`, Playwright's
 * server `WebMCP`). A tool's input goes out as Playwright's `serializeValue` writes it, with no
 * handles (`plain-argument.ts`), and its answer comes back through `parseResult`.
 */

import type { FrameWebmcpToolsResult } from '@browseros/neo-runtime-spec/serve'
import type { Frame } from './frame'
import { parseResult } from './js-handle'
import { serializePlainArgument } from './plain-argument'

/** Playwright's `WebMCPTool`: one tool as `tools()` lists it. */
export type WebMCPTool = FrameWebmcpToolsResult['tools'][number]

export class WebMCP {
  readonly #frame: Frame

  constructor(frame: Frame) {
    this.#frame = frame
  }

  async tools(options: { timeout?: number } = {}): Promise<WebMCPTool[]> {
    const result = (await this.#frame._connection.send(
      'webMCP.tools',
      this.#frame._guid,
      'webmcpTools',
      {},
      this.#frame._page._timeoutSettings.timeout(options, 'webMCP.tools'),
    )) as FrameWebmcpToolsResult
    return result.tools
  }

  async callTool(
    name: string,
    input?: unknown,
    options: { timeout?: number } = {},
  ): Promise<unknown> {
    const result = (await this.#frame._connection.send(
      'webMCP.callTool',
      this.#frame._guid,
      'webmcpCallTool',
      { name, input: serializePlainArgument(input) },
      this.#frame._page._timeoutSettings.timeout(options, 'webMCP.callTool'),
    )) as { result: unknown }
    return parseResult(result.result)
  }
}
