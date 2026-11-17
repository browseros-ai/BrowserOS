import type { RuledCaseSpec } from '../model'
import { interactionsCases } from './interactions'
import { readsCases } from './reads'
import { specialCases } from './special'

/** One authored catalog is composed once with the frozen historical registry. */
export const ruledCases: readonly RuledCaseSpec[] = [
  ...interactionsCases,
  ...readsCases,
  ...specialCases,
].sort((a, b) => a.file.localeCompare(b.file) || a.id.localeCompare(b.id))
