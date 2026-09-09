import type { LlmProviderConfig } from './types'

/**
 * @public
 */
export interface TestResult {
  success: boolean
  message: string
  responseTime?: number
  fieldErrors?: Record<string, string>
}

/** Saved tests use only the ID; cached UI fields never override the saved configuration. */
export function testProvider(
  providerId: string,
  agentServerUrl: string,
): Promise<TestResult> {
  return requestProviderTest({ providerId }, agentServerUrl)
}

/** A draft carries user edits; the server resolves defaults and stored credentials. */
export function testProviderDraft(
  provider: Omit<LlmProviderConfig, 'id'> & { id?: string },
  agentServerUrl: string,
): Promise<TestResult> {
  return requestProviderTest(
    {
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
    },
    agentServerUrl,
  )
}

async function requestProviderTest(
  body: Record<string, unknown>,
  agentServerUrl: string,
): Promise<TestResult> {
  const startTime = performance.now()

  try {
    const response = await fetch(`${agentServerUrl}/test-provider`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })

    const result = (await response.json()) as TestResult

    if (typeof result.message !== 'string') {
      return {
        success: false,
        message:
          'The BrowserOS server could not test these settings. Update BrowserOS and try again.',
      }
    }
    if (!result.responseTime) {
      result.responseTime = Math.round(performance.now() - startTime)
    }

    return result
  } catch (error) {
    // Any throw at this layer means the client could not complete the
    // round-trip to the local BrowserOS server that hosts
    // /test-provider (network failure, CORS, response body not
    // JSON, ...). Server validation and upstream test failures return a
    // message in the response body via the happy path above; they never
    // reach this catch. Distinguish
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
