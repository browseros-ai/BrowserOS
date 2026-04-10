import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import OpenAI from 'openai'
import type { ChatCompletionContentPart } from 'openai/resources/chat/completions'
import type { GraderResult } from '../../types'
import type { Grader, GraderInput } from '../types'

/**
 * Fara Multimodal Verifier
 *
 * Based on the Fara paper (Microsoft Research, 2024):
 * "This verifier inspects the screenshots and final response of the trajectory
 * to check whether the task was successfully completed. The verifier first selects
 * the most relevant screenshots from the trajectory based on the task ranked by
 * how informative they are."
 *
 * Two-phase evaluation:
 * 1. Select most relevant screenshots based on task relevance
 * 2. Judge:
 *    a) Whether the final response is fully consistent with screenshot evidence
 *    b) Whether the content in screenshots appears to satisfy the task
 *
 * "The Multimodal Verifier is especially important for combating hallucinations."
 */

const SCREENSHOT_SELECTION_PROMPT = `You are an expert evaluator selecting the most relevant screenshots from a web agent's trajectory.

**Instructions:**
1. You will see multiple screenshots from an agent's web navigation
2. Score each screenshot from 1-5 based on relevance to the task:
   - 1: Not relevant at all
   - 2: Minimal relevance
   - 3: Somewhat relevant
   - 4: Highly relevant
   - 5: Critical/essential for verifying task completion

**Output Format:**
Return a JSON object:
{
  "scores": [
    {"index": <1-based index>, "score": <1-5>, "reason": "Brief reason"}
  ]
}`

const MULTIMODAL_VERIFICATION_PROMPT = `You are an expert evaluator verifying web agent task completion using visual evidence.

**Your role is to verify two critical aspects:**

1. **Response-Screenshot Consistency**: Is the agent's final response fully consistent with what is shown in the screenshots?
   - Does the response accurately describe information visible in screenshots?
   - Are there any claims in the response not supported by visual evidence?
   - Look for hallucinations - information the agent claims but cannot be verified

2. **Task Completion Evidence**: Do the screenshots show evidence that the task was successfully completed?
   - Can you see the target page, information, or action result?
   - Is there visual confirmation of the requested action/information?
   - For search tasks: are correct search results visible?
   - For navigation tasks: did the agent reach the target page?
   - For information tasks: is the answer visible on screen?

**Important:** The Multimodal Verifier is especially important for combating hallucinations. Be skeptical of claims not supported by visual evidence.

**Output Format:**
Provide your analysis, then conclude with:

RESPONSE_CONSISTENT: YES or NO
TASK_SATISFIED: YES or NO
VERDICT: PASS or FAIL
REASONING: <One sentence summary>`

interface ScreenshotScore {
  index: number
  score: number
  reason: string
}

export class FaraMultimodalGrader implements Grader {
  name = 'fara_multimodal'
  private client: OpenAI
  private model: string
  private relevanceThreshold = 3
  private maxSelectedScreenshots = 10
  private maxEvaluationScreenshots = 30
  private maxRetries = 3
  private retryDelayMs = 1000

  constructor(apiKey: string, baseUrl?: string, model?: string) {
    this.client = new OpenAI({
      apiKey,
      baseURL: baseUrl || undefined,
    })
    this.model = model || 'gpt-4o'
  }

  async grade(input: GraderInput): Promise<GraderResult> {
    try {
      // Load available screenshots
      const allScreenshots = await this.loadScreenshots(
        input.outputDir,
        input.screenshotCount,
      )

      if (allScreenshots.length === 0) {
        return {
          score: 0,
          pass: false,
          reasoning: 'No screenshots available for multimodal verification',
          details: { verifier: 'multimodal', error: 'no_screenshots' },
        }
      }

      // Step 1: Select most relevant screenshots
      const selectedScreenshots = await this.selectRelevantScreenshots(
        input.task.query,
        allScreenshots,
      )

      if (selectedScreenshots.length === 0) {
        return {
          score: 0,
          pass: false,
          reasoning:
            'No relevant screenshots found for verification. All screenshots scored below relevance threshold.',
          details: {
            verifier: 'multimodal',
            totalScreenshots: allScreenshots.length,
            relevantScreenshots: 0,
            threshold: this.relevanceThreshold,
          },
        }
      }

      // Step 2: Verify task completion with selected screenshots
      const verification = await this.verifyWithScreenshots(
        input.task.query,
        input.finalAnswer,
        selectedScreenshots,
      )

      const isPass =
        verification.responseConsistent && verification.taskSatisfied

      return {
        score: isPass ? 1 : 0,
        pass: isPass,
        reasoning: verification.fullReasoning,
        details: {
          verifier: 'multimodal',
          totalScreenshots: allScreenshots.length,
          relevantScreenshots: selectedScreenshots.length,
          selectedIndices: selectedScreenshots.map((s) => s.index),
          responseConsistent: verification.responseConsistent,
          taskSatisfied: verification.taskSatisfied,
          model: this.model,
        },
      }
    } catch (error) {
      return {
        score: 0,
        pass: false,
        reasoning: `Multimodal verifier error: ${error instanceof Error ? error.message : String(error)}`,
        details: { error: true, verifier: 'multimodal' },
      }
    }
  }

  private async loadScreenshots(
    outputDir: string,
    screenshotCount: number,
  ): Promise<{ index: number; data: string }[]> {
    const screenshots: { index: number; data: string }[] = []

    // Sample screenshots if too many
    const indices: number[] = []
    if (screenshotCount <= this.maxEvaluationScreenshots) {
      for (let i = 1; i <= screenshotCount; i++) {
        indices.push(i)
      }
    } else {
      // Sample evenly across the trajectory, always include first, last, and recent
      const step = Math.floor(screenshotCount / this.maxEvaluationScreenshots)
      for (let i = 1; i <= screenshotCount; i += step) {
        indices.push(i)
      }
      // Always include the last few screenshots (most likely to show completion)
      for (let i = screenshotCount - 4; i <= screenshotCount; i++) {
        if (i > 0 && !indices.includes(i)) {
          indices.push(i)
        }
      }
      indices.sort((a, b) => a - b)
    }

    const loadPromises = []
    for (const i of indices) {
      const filepath = join(outputDir, 'screenshots', `${i}.png`)
      loadPromises.push(
        readFile(filepath)
          .then((buffer) => {
            const base64 = buffer.toString('base64')
            return {
              index: i,
              data: `data:image/png;base64,${base64}`,
            }
          })
          .catch(() => null), // Skip missing files
      )
    }

    const results = await Promise.all(loadPromises)
    for (const result of results) {
      if (result) {
        screenshots.push(result)
      }
    }

    return screenshots
  }

  private async selectRelevantScreenshots(
    task: string,
    screenshots: { index: number; data: string }[],
  ): Promise<{ index: number; data: string; score: number }[]> {
    if (screenshots.length <= this.maxSelectedScreenshots) {
      return screenshots.map((s) => ({ ...s, score: 5 }))
    }

    // Use batched evaluation to score screenshots
    const batchSize = 5
    const allScores: ScreenshotScore[] = []

    for (let i = 0; i < screenshots.length; i += batchSize) {
      const batch = screenshots.slice(i, i + batchSize)
      const scores = await this.scoreScreenshotBatch(task, batch, i)
      allScores.push(...scores)
    }

    // Filter by threshold and sort by score
    const relevant = allScores
      .filter((s) => s.score >= this.relevanceThreshold)
      .sort((a, b) => b.score - a.score)
      .slice(0, this.maxSelectedScreenshots)

    // If not enough relevant screenshots, include the highest scored ones anyway
    if (relevant.length < 3 && allScores.length > 0) {
      const topScores = allScores
        .sort((a, b) => b.score - a.score)
        .slice(0, Math.min(5, allScores.length))

      for (const score of topScores) {
        if (!relevant.find((r) => r.index === score.index)) {
          relevant.push(score)
        }
      }
    }

    return relevant.map((score) => ({
      index: score.index,
      data: screenshots.find((s) => s.index === score.index)?.data ?? '',
      score: score.score,
    }))
  }

  private async scoreScreenshotBatch(
    task: string,
    batch: { index: number; data: string }[],
    _startOffset: number,
  ): Promise<ScreenshotScore[]> {
    const content: ChatCompletionContentPart[] = [
      {
        type: 'text',
        text: `Task: ${task}\n\nScore the following ${batch.length} screenshots for relevance to this task. Screenshots are numbered ${batch[0].index} to ${batch[batch.length - 1].index}.`,
      },
    ]

    for (const screenshot of batch) {
      content.push({
        type: 'text',
        text: `\n--- Screenshot ${screenshot.index} ---`,
      })
      content.push({
        type: 'image_url',
        image_url: { url: screenshot.data, detail: 'low' },
      })
    }

    try {
      const response = await this.callWithRetry(
        [
          { role: 'system', content: SCREENSHOT_SELECTION_PROMPT },
          { role: 'user', content },
        ],
        true,
      )

      const responseContent = response.choices[0]?.message?.content || ''
      return this.parseScreenshotScores(responseContent, batch)
    } catch {
      // On error, give all screenshots average score
      return batch.map((s) => ({
        index: s.index,
        score: 3,
        reason: 'Could not evaluate',
      }))
    }
  }

  private parseScreenshotScores(
    content: string,
    batch: { index: number; data: string }[],
  ): ScreenshotScore[] {
    try {
      const jsonMatch = content.match(/\{[\s\S]*\}/)
      if (jsonMatch) {
        const parsed = JSON.parse(jsonMatch[0])
        if (parsed.scores && Array.isArray(parsed.scores)) {
          return parsed.scores.map((s: Partial<ScreenshotScore>) => ({
            index: s.index ?? batch[0].index,
            score: Math.min(5, Math.max(1, s.score ?? 3)),
            reason: s.reason ?? 'No reason provided',
          }))
        }
      }
    } catch {
      // Fall through
    }

    // Default scores
    return batch.map((s) => ({
      index: s.index,
      score: 3,
      reason: 'Could not parse score',
    }))
  }

  private async verifyWithScreenshots(
    task: string,
    finalAnswer: string | null,
    screenshots: { index: number; data: string; score: number }[],
  ): Promise<{
    responseConsistent: boolean
    taskSatisfied: boolean
    fullReasoning: string
  }> {
    const content: ChatCompletionContentPart[] = [
      {
        type: 'text',
        text: `**Task:** ${task}\n\n**Agent's Final Response:** ${finalAnswer || '[No response provided]'}\n\n**Selected Screenshots (${screenshots.length} most relevant):**`,
      },
    ]

    for (const screenshot of screenshots) {
      content.push({
        type: 'text',
        text: `\n--- Screenshot ${screenshot.index} (relevance score: ${screenshot.score}/5) ---`,
      })
      content.push({
        type: 'image_url',
        image_url: { url: screenshot.data, detail: 'high' },
      })
    }

    content.push({
      type: 'text',
      text: '\nVerify the task completion based on the screenshots and final response.',
    })

    const response = await this.callWithRetry([
      { role: 'system', content: MULTIMODAL_VERIFICATION_PROMPT },
      { role: 'user', content },
    ])

    const responseContent = response.choices[0]?.message?.content || ''
    return this.parseVerification(responseContent)
  }

  private parseVerification(content: string): {
    responseConsistent: boolean
    taskSatisfied: boolean
    fullReasoning: string
  } {
    const upperContent = content.toUpperCase()

    // Parse RESPONSE_CONSISTENT
    let responseConsistent = false
    if (upperContent.includes('RESPONSE_CONSISTENT: YES')) {
      responseConsistent = true
    } else if (upperContent.includes('RESPONSE_CONSISTENT: NO')) {
      responseConsistent = false
    } else {
      // Fallback: check if there's any indication
      responseConsistent =
        !upperContent.includes('HALLUCINATION') &&
        !upperContent.includes('INCONSISTENT') &&
        !upperContent.includes('NOT SUPPORTED')
    }

    // Parse TASK_SATISFIED
    let taskSatisfied = false
    if (upperContent.includes('TASK_SATISFIED: YES')) {
      taskSatisfied = true
    } else if (upperContent.includes('TASK_SATISFIED: NO')) {
      taskSatisfied = false
    } else {
      // Fallback: check verdict
      if (upperContent.includes('VERDICT: PASS')) {
        taskSatisfied = true
      }
    }

    // Override with final verdict if present
    if (upperContent.includes('VERDICT: FAIL')) {
      // If explicit fail, at least one criterion failed
      if (
        !upperContent.includes('RESPONSE_CONSISTENT:') &&
        !upperContent.includes('TASK_SATISFIED:')
      ) {
        responseConsistent = false
        taskSatisfied = false
      }
    }

    return {
      responseConsistent,
      taskSatisfied,
      fullReasoning: content,
    }
  }

  private async callWithRetry(
    messages: OpenAI.Chat.Completions.ChatCompletionMessageParam[],
    useJsonFormat = false,
    attempt = 1,
  ): Promise<OpenAI.Chat.Completions.ChatCompletion> {
    try {
      const options: OpenAI.Chat.Completions.ChatCompletionCreateParamsNonStreaming =
        {
          model: this.model,
          temperature: 0,
          messages,
          max_tokens: 2000,
        }

      if (useJsonFormat) {
        options.response_format = { type: 'json_object' }
      }

      return await this.client.chat.completions.create(options)
    } catch (error) {
      if (attempt < this.maxRetries) {
        const delay = this.retryDelayMs * 2 ** (attempt - 1)
        await new Promise((resolve) => setTimeout(resolve, delay))
        return this.callWithRetry(messages, useJsonFormat, attempt + 1)
      }
      throw error
    }
  }
}
