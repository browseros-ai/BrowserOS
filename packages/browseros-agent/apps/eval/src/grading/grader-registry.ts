import { AgisdkStateDiffGrader } from '../graders/benchmark/agisdk-state-diff'
import { InfinityStateGrader } from '../graders/benchmark/infinity-state'
import { PerformanceGrader } from '../graders/performance/performance-grader'
import type { Grader } from './types'

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
