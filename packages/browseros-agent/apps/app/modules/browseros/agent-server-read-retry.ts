export const AGENT_SERVER_READ_RETRY_LIMIT = 3

/** Keeps startup reads pending while the browser opens the panel ahead of the local server. */
export const agentServerReadRetry = {
  retry: (failureCount: number, error: unknown) =>
    // Chromium reports a connection refusal as a fetch TypeError. Keep the
    // normal three retries for HTTP errors and errors decoding a response.
    (error instanceof TypeError && error.message === 'Failed to fetch') ||
    failureCount < AGENT_SERVER_READ_RETRY_LIMIT,
  retryDelay: (attempt: number) => Math.min(1000 * 2 ** attempt, 5000),
}
