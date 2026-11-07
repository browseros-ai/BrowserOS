import type { Page } from '../../src/page'

export async function snapshotSize(page: Page) {
  const nodes = await page.ariaSnapshotJSON()
  return nodes.length
}
