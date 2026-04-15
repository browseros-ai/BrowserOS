import { describe, expect, it } from 'bun:test'
import { getModelsForProvider } from './models'

describe('getModelsForProvider', () => {
  describe('qwen-code', () => {
    it('includes supported models', () => {
      const models = getModelsForProvider('qwen-code')
      const ids = models.map((m) => m.modelId)
      expect(ids).toContain('coder-model')
      expect(ids).toContain('qwen3-coder-plus')
      expect(ids).toContain('qwen3-coder-flash')
    })

    it('does not include qwen3.5-plus which is unsupported', () => {
      const models = getModelsForProvider('qwen-code')
      const ids = models.map((m) => m.modelId)
      expect(ids).not.toContain('qwen3.5-plus')
    })
  })
})
