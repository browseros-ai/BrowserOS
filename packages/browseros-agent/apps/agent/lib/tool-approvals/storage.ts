import { storage } from '@wxt-dev/storage'
import type { ToolApprovalCategoryId, ToolApprovalConfig } from './types'

export const toolApprovalConfigStorage = storage.defineItem<ToolApprovalConfig>(
  'local:tool-approval-config',
  {
    fallback: {
      categories: {},
    },
  },
)

const LEGACY_ALL_CATEGORY_IDS: ToolApprovalCategoryId[] = [
  'input',
  'navigation',
  'screenshots',
  'scripts',
  'data-modification',
]

const NEW_CATEGORY_IDS: ToolApprovalCategoryId[] = ['observation', 'assistant']

export function normalizeToolApprovalConfig(
  config: ToolApprovalConfig,
): ToolApprovalConfig {
  const categories = { ...config.categories }
  const shouldMigrateLegacyAll =
    LEGACY_ALL_CATEGORY_IDS.every((id) => categories[id] === true) &&
    NEW_CATEGORY_IDS.every((id) => categories[id] === undefined)

  if (shouldMigrateLegacyAll) {
    for (const id of NEW_CATEGORY_IDS) {
      categories[id] = true
    }
  }

  return { categories }
}
