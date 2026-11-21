import { ServerApiClient } from '@browseros/server-api/client'
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
    const result = await new ServerApiClient(agentServerUrl).testProvider({
      // Lets the server fill a blank key from this provider's saved,
      // redacted credential when testing an existing provider.
      providerId: provider.id,
      provider: provider.type,
      model: provider.modelId,
      apiKey: provider.apiKey,
      baseUrl: provider.baseUrl,
      headers: provider.headers,
      // Azure
      resourceName: provider.resourceName,
      // Bedrock
      region: provider.region,
      accessKeyId: provider.accessKeyId,
      secretAccessKey: provider.secretAccessKey,
      sessionToken: provider.sessionToken,
    })
    return {
      ...result,
      responseTime:
        result.responseTime ?? Math.round(performance.now() - startTime),
    }
  } catch (error) {
    // Provider errors arrive as test results above. Keep failures to get a
    // usable result tied to the local server URL so they aren't mistaken for
    // errors in the user's provider endpoint (issue #1844).
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
