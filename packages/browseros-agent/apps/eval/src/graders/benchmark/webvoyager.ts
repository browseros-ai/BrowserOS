import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import OpenAI from 'openai'
import type { GraderResult } from '../../types'
import type { Grader, GraderInput } from '../types'

/**
 * WebVoyager Grader - Exact implementation based on original WebVoyager auto_eval.py
 * Reference: https://github.com/MinorJerry/WebVoyager/blob/main/evaluation/auto_eval.py
 *
 * Uses GPT-4V to evaluate task completion by analyzing screenshots and final response.
 */

const WEBVOYAGER_SYSTEM_PROMPT = `As an evaluator, you will be presented with three primary components to assist you in your role:

1. Web Task Instruction: This is a clear and specific directive provided in natural language, detailing the online activity to be carried out. These requirements may include conducting searches, verifying information, comparing prices, checking availability, or any other action relevant to the specified web service (such as Amazon, Apple, ArXiv, BBC News, Booking etc).

2. Result Screenshots: This is a visual representation of the screen showing the result or intermediate state of performing a web task. It serves as visual proof of the actions taken in response to the instruction.

3. Result Response: This is a textual response obtained after the execution of the web task. It serves as textual result in response to the instruction.

-- You DO NOT NEED to interact with web pages or perform actions such as booking flights or conducting searches on websites.
-- You SHOULD NOT make assumptions based on information not presented in the screenshot when comparing it to the instructions.
-- Your primary responsibility is to conduct a thorough assessment of the web task instruction against the outcome depicted in the screenshot and in the response, evaluating whether the actions taken align with the given instructions.
-- NOTE that the instruction may involve more than one task, for example, locating the garage and summarizing the review. Failing to complete either task, such as not providing a summary, should be considered unsuccessful.
-- NOTE that the screenshot is authentic, but the response provided by LLM is generated at the end of web browsing, and there may be discrepancies between the text and the screenshots.
-- Note the difference: 1) Result response may contradict the screenshot, then the content of the screenshot prevails, 2) The content in the Result response is not mentioned on the screenshot, choose to believe the content.

You should elaborate on how you arrived at your final evaluation and then provide a definitive verdict on whether the task has been successfully accomplished, either as 'SUCCESS' or 'NOT SUCCESS'.`

export class WebVoyagerGrader implements Grader {
  name = 'webvoyager_grader'
  private client: OpenAI
  private maxScreenshots = 15
  private model: string

  constructor(apiKey: string, baseURL?: string, model?: string) {
    this.client = new OpenAI({
      apiKey,
      baseURL: baseURL || undefined,
    })
    this.model = model || 'gpt-4o'
  }

  async grade(input: GraderInput): Promise<GraderResult> {
    // Load screenshots (last N screenshots)
    const startNum = Math.max(
      1,
      input.screenshotCount - this.maxScreenshots + 1,
    )
    const endNum = input.screenshotCount

    const images: { type: 'image_url'; image_url: { url: string } }[] = []
    const loadedScreenshots: number[] = []

    const loadPromises = []
    for (let i = startNum; i <= endNum; i++) {
      const filepath = join(input.outputDir, 'screenshots', `${i}.png`)
      loadPromises.push(
        readFile(filepath)
          .then((buffer) => {
            const base64 = buffer.toString('base64')
            return {
              index: i,
              image: {
                type: 'image_url' as const,
                image_url: { url: `data:image/png;base64,${base64}` },
              },
            }
          })
          .catch(() => null), // Skip missing files
      )
    }

    const results = await Promise.all(loadPromises)
    for (const result of results) {
      if (result) {
        images.push(result.image)
        loadedScreenshots.push(result.index)
      }
    }

    if (images.length === 0) {
      return {
        score: 0,
        pass: false,
        reasoning: 'No screenshots available for evaluation',
      }
    }

    // Build user prompt (matching original WebVoyager format)
    const userPrompt = `TASK: ${input.task.query}
Result Response: ${input.finalAnswer || '[No response provided]'}
${images.length} screenshots at the end:`

    try {
      const response = await this.client.chat.completions.create({
        model: this.model,
        temperature: 0,
        seed: 42,
        messages: [
          { role: 'system', content: WEBVOYAGER_SYSTEM_PROMPT },
          {
            role: 'user',
            content: [
              { type: 'text', text: userPrompt },
              ...images,
              { type: 'text', text: 'Your verdict:\n' },
            ],
          },
        ],
        max_tokens: 1000,
      })

      const content = response.choices[0]?.message?.content || ''

      // Parse verdict (matching original logic)
      // "NOT SUCCESS" must be checked first as it contains "SUCCESS"
      let isSuccess: boolean
      if (content.toUpperCase().includes('NOT SUCCESS')) {
        isSuccess = false
      } else if (content.toUpperCase().includes('SUCCESS')) {
        isSuccess = true
      } else {
        // Ambiguous response - default to failure
        isSuccess = false
      }

      return {
        score: isSuccess ? 1 : 0,
        pass: isSuccess,
        reasoning: content,
        details: {
          screenshotsEvaluated: images.length,
          screenshotRange: `${loadedScreenshots[0]}-${loadedScreenshots[loadedScreenshots.length - 1]}`,
          model: this.model,
          promptTokens: response.usage?.prompt_tokens,
          completionTokens: response.usage?.completion_tokens,
        },
      }
    } catch (error) {
      return {
        score: 0,
        pass: false,
        reasoning: `Grader error: ${error instanceof Error ? error.message : String(error)}`,
        details: { error: true },
      }
    }
  }
}
