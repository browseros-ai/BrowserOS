import type { GraderResult } from '../types'
import { AgisdkStateDiffGrader } from './benchmark/agisdk-state-diff'
import { InfinityStateGrader } from './benchmark/infinity-state'
import { Mind2WebJudgeGrader } from './benchmark/mind2web'
import { WebVoyagerGrader } from './benchmark/webvoyager'
import { FaraAlignmentGrader } from './fara/alignment'
import { FaraCombinedGrader } from './fara/combined'
import { FaraMultimodalGrader } from './fara/multimodal'
import { FaraRubricGrader } from './fara/rubric'
import { PerformanceGrader } from './performance/performance-grader'
import type { Grader, GraderInput } from './types'

interface GraderOptions {
  apiKey: string
  baseUrl?: string
  model?: string
}

export function createGrader(
  name: string,
  options: GraderOptions | null,
): Grader | null {
  switch (name) {
    // Deterministic benchmark graders (no LLM judge)
    case 'agisdk_state_diff':
      return new AgisdkStateDiffGrader()
    case 'infinity_state':
      return new InfinityStateGrader()

    // LLM-based benchmark graders
    case 'webvoyager_grader':
      if (!options?.apiKey) return null
      return new WebVoyagerGrader(
        options.apiKey,
        options.baseUrl,
        options.model,
      )
    case 'mind2web_judge':
    case 'mind2web_grader':
      if (!options?.apiKey) return null
      return new Mind2WebJudgeGrader(
        options.apiKey,
        options.baseUrl,
        options.model,
      )

    // Fara individual verifiers
    case 'fara_alignment':
      if (!options?.apiKey) return null
      return new FaraAlignmentGrader(
        options.apiKey,
        options.baseUrl,
        options.model || 'gpt-4o-mini',
      )
    case 'fara_rubric':
      if (!options?.apiKey) return null
      return new FaraRubricGrader(
        options.apiKey,
        options.baseUrl,
        options.model || 'gpt-4o-mini',
      )
    case 'fara_multimodal':
      if (!options?.apiKey) return null
      return new FaraMultimodalGrader(
        options.apiKey,
        options.baseUrl,
        options.model || 'gpt-4o',
      )

    // Fara combined 3-verifier system (majority voting)
    case 'fara_grader':
    case 'fara_combined':
      if (!options?.apiKey) return null
      return new FaraCombinedGrader(
        options.apiKey,
        options.baseUrl,
        options.model,
      )

    // Multi-axis performance grader (Claude Agent SDK — uses its own Claude default model)
    case 'performance_grader':
      return new PerformanceGrader()

    default:
      console.warn(`Unknown grader: ${name}`)
      return null
  }
}

export async function runGraders(
  graderNames: string[],
  input: GraderInput,
  options: GraderOptions | null,
): Promise<Record<string, GraderResult>> {
  const results: Record<string, GraderResult> = {}

  for (const name of graderNames) {
    const grader = createGrader(name, options)
    if (grader) {
      try {
        console.log(`  Running grader: ${name}`)
        results[name] = await grader.grade(input)
      } catch (error) {
        results[name] = {
          score: 0,
          pass: false,
          reasoning: `Error running grader: ${error}`,
        }
      }
    }
  }

  return results
}

// Export grader classes for direct use
export {
  AgisdkStateDiffGrader,
  FaraAlignmentGrader,
  FaraCombinedGrader,
  FaraMultimodalGrader,
  FaraRubricGrader,
  InfinityStateGrader,
  Mind2WebJudgeGrader,
  PerformanceGrader,
  WebVoyagerGrader,
}
