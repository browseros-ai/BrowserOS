import type { LlmProviderConfig } from './types'

/**
 * @public
 */
export interface TestResult {
  success: boolean
  message: string
  responseTime?: number
}

/**
 * Test a provider connection via the agent server's /test-provider endpoint.
 * This uses the same code path as actual chat requests, ensuring accurate validation.
 * @public
 */
export async function testProvider(
  provider: LlmProviderConfig,
  agentServerUrl: string,
): Promise<TestResult> {
  const startTime = performance.now()

  try {
    const response = await fetch(`${agentServerUrl}/test-provider`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        provider: provider.type,
        model: provider.modelId,
        apiKey: provider.apiKey,
        baseUrl: provider.baseUrl,
        // Azure
        resourceName: provider.resourceName,
        // Bedrock
        region: provider.region,
        accessKeyId: provider.accessKeyId,
        secretAccessKey: provider.secretAccessKey,
        sessionToken: provider.sessionToken,
        // ACP-backed providers reach the probe via the same endpoint.
        acpAgentId: provider.acpAgentId,
        acpCommand: provider.acpCommand,
        acpFixedWorkspacePath: provider.acpFixedWorkspacePath,
      }),
    })

    const result = (await response.json()) as TestResult

    if (!result.responseTime) {
      result.responseTime = Math.round(performance.now() - startTime)
    }

    return result
  } catch (error) {
    // Any throw at this layer means the client could not complete the
    // round-trip to the local BrowserOS server that hosts
    // /test-provider (network failure, CORS, response body not
    // JSON, ...). Server-side test failures are prefixed with the
    // provider name inside the response body and are returned via
    // the happy path above; they never reach this catch. Distinguish
    // the two so users don't read "Failed to fetch (127.0.0.1:9200)"
    // as "BrowserOS dropped the port I typed" (see issue #1844).
    const responseTime = Math.round(performance.now() - startTime)
    const detail = error instanceof Error ? error.message : String(error)

    return {
      success: false,
      message:
        `Could not reach the local BrowserOS server at ${agentServerUrl}. ` +
        `Make sure BrowserOS is running and try again. (${detail})`,
      responseTime,
    }
  }
}
