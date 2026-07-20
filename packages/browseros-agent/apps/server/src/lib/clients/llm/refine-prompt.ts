import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { LLMConfig } from '@browseros/shared/schemas/llm'
import { streamText } from 'ai'
import { resolveLLMConfig } from './config'
import { createLLMProvider } from './provider'

export interface RefinePromptConfig extends LLMConfig {
  model: string
  upstreamProvider?: string
}

export interface RefinePromptRequest {
  prompt: string
  name: string
}

export interface RefinePromptResult {
  success: boolean
  refined?: string
  message?: string
}

function buildSystemPrompt(name: string): string {
  return `You are helping a user write a prompt for a scheduled browser automation task called "${name}".

This prompt will be executed automatically on a recurring schedule by an AI agent that can fully control a browser — navigate sites, click, type, read content, and take screenshots.

Rewrite the user's rough prompt into a clear, natural instruction. Make it:
- Specific about what to do and where (which websites, what pages, what to look for)
- Clear about what result to return at the end (a summary, key data points, changes detected, etc.)
- Complete enough to run unattended — the agent can't ask follow-up questions

If the user's prompt is too vague to fill in specifics, use natural placeholders like [your competitor's URL] that they can easily spot and replace.

Write it as a natural instruction — like telling a capable assistant what to do. Keep it concise. Return ONLY the rewritten prompt, nothing else.`
}

export async function refinePrompt(
  llmConfig: RefinePromptConfig,
  request: RefinePromptRequest,
  browserosId?: string,
): Promise<RefinePromptResult> {
  try {
    const resolvedConfig = await resolveLLMConfig(llmConfig, browserosId)
    const model = createLLMProvider(resolvedConfig)

    // streamText works for all providers including Codex (which requires streaming).
    // Capture streaming errors: the SDK's default onError just logs to
    // console and does not propagate, so provider failures (bad URL,
    // auth, timeout) end up as internal error chunks that `textStream`
    // filters out. Without capturing, the loop exits with zero chunks
    // and we misreport the failure as "Provider returned an empty
    // response". Re-throw after the loop so the catch below surfaces
    // the real error message.
    let capturedError: unknown = null
    const stream = streamText({
      model,
      system: buildSystemPrompt(request.name),
      messages: [{ role: 'user', content: request.prompt }],
      abortSignal: AbortSignal.timeout(TIMEOUTS.REFINE_PROMPT),
      onError: ({ error }) => {
        capturedError = error
      },
    })
    const chunks: string[] = []
    for await (const chunk of stream.textStream) {
      chunks.push(chunk)
    }
    if (capturedError) throw capturedError
    const refined = chunks.join('').trim()

    if (!refined) {
      return { success: false, message: 'Provider returned an empty response' }
    }

    return { success: true, refined }
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error)
    return { success: false, message: errorMessage }
  }
}
