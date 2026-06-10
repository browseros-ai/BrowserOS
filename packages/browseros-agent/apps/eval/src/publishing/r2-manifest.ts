import type { ViewerManifest } from '../viewer/viewer-manifest'

export interface R2UploadConfig {
  accountId: string
  accessKeyId: string
  secretAccessKey: string
  bucket: string
  cdnBaseUrl: string
}

export type R2RunManifest = ViewerManifest

export interface R2PublishRunResult {
  runId: string
  uploadedFiles: number
  viewerUrl: string
  manifest: R2RunManifest
}

export interface R2PublishPathResult {
  uploadedRuns: R2PublishRunResult[]
  skippedRuns: string[]
}
