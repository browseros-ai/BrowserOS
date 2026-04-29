import type { GraderResult } from '../types'
import { AgisdkStateDiffGrader } from './benchmark/agisdk-state-diff'
import { InfinityStateGrader } from './benchmark/infinity-state'
import { PerformanceGrader } from './performance/performance-grader'
import type { Grader, GraderInput } from './types'

export const PASS_FAIL_GRADER_ORDER = [
  'agisdk_state_diff',
  'infinity_state',
  'performance_grader',
] as const

export function createGrader(name: string): Grader | null {
  switch (name) {
    case 'agisdk_state_diff':
      return new AgisdkStateDiffGrader()
    case 'infinity_state':
      return new InfinityStateGrader()
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
): Promise<Record<string, GraderResult>> {
  const results: Record<string, GraderResult> = {}

  for (const name of graderNames) {
    const grader = createGrader(name)
    if (!grader) continue
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

  return results
}

export { AgisdkStateDiffGrader, InfinityStateGrader, PerformanceGrader }
