import type { GraderResult, Message } from '../types'

export interface GraderInput {
  task: {
    query_id: string
    query: string
    dataset: string
  }
  messages: Message[]
  screenshotCount: number
  finalAnswer: string | null
  expectedAnswer?: string | null
  outputDir: string
  mcpUrl?: string
  infinityAppUrl?: string
}

export interface Grader {
  name: string
  grade(input: GraderInput): Promise<GraderResult>
}
