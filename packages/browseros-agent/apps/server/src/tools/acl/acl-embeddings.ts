import { logger } from '../../lib/logger'

interface SemanticScore {
  score: number
  backend: string
}

type FeatureExtractionPipeline = (
  texts: string[],
  options: { pooling: string; normalize: boolean },
) => Promise<{ tolist: () => number[][] }>

let pipelineInstance: FeatureExtractionPipeline | null = null
const LOAD_RETRY_MS = 60_000
let lastLoadFailedAt = 0

function getModelName(): string {
  return process.env.ACL_EMBEDDING_MODEL ?? 'Xenova/bge-small-en-v1.5'
}

async function ensurePipeline(): Promise<FeatureExtractionPipeline | null> {
  if (pipelineInstance) return pipelineInstance
  if (lastLoadFailedAt > 0 && Date.now() - lastLoadFailedAt < LOAD_RETRY_MS) {
    return null
  }

  try {
    const { pipeline } = await import('@huggingface/transformers')
    const extractor = await pipeline('feature-extraction', getModelName(), {
      dtype: 'fp32',
    })
    pipelineInstance = extractor as unknown as FeatureExtractionPipeline
    lastLoadFailedAt = 0
    logger.info('ACL embedding model loaded', { model: getModelName() })
    return pipelineInstance
  } catch (error) {
    lastLoadFailedAt = Date.now()
    logger.warn(
      'ACL embedding model failed to load, semantic scoring disabled',
      {
        model: getModelName(),
        error: error instanceof Error ? error.message : String(error),
      },
    )
    return null
  }
}

function cosineSimilarity(a: number[], b: number[]): number {
  let dot = 0
  let normA = 0
  let normB = 0
  for (let i = 0; i < a.length; i++) {
    dot += a[i] * b[i]
    normA += a[i] * a[i]
    normB += b[i] * b[i]
  }
  const denom = Math.sqrt(normA) * Math.sqrt(normB)
  return denom === 0 ? 0 : dot / denom
}

export async function computeSemanticSimilarity(
  left: string,
  right: string,
): Promise<SemanticScore> {
  if (!left || !right) return { score: 0, backend: 'none' }

  const extractor = await ensurePipeline()
  if (!extractor) return { score: 0, backend: 'error' }

  try {
    const output = await extractor([left, right], {
      pooling: 'cls',
      normalize: true,
    })
    const embeddings = output.tolist()
    const score = cosineSimilarity(embeddings[0], embeddings[1])
    return {
      score: Math.max(0, Math.min(score, 1)),
      backend: 'transformers.js',
    }
  } catch (error) {
    logger.warn('ACL semantic similarity computation failed', {
      error: error instanceof Error ? error.message : String(error),
    })
    return { score: 0, backend: 'error' }
  }
}
