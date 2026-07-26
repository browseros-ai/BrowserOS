import type { BunPlugin } from 'bun'

export type TargetId =
  | 'linux-x64'
  | 'linux-arm64'
  | 'windows-x64'
  | 'darwin-arm64'
  | 'darwin-x64'

export type TargetOs = 'linux' | 'macos' | 'windows'
export type TargetArch = 'x64' | 'arm64'

export interface BuildTarget {
  id: TargetId
  name: string
  os: TargetOs
  arch: TargetArch
  bunTarget: string
}

export interface BuildArgs {
  targets: BuildTarget[]
  manifestPath: string
  upload: boolean
  ci: boolean
}

export interface R2Config {
  accountId: string
  accessKeyId: string
  secretAccessKey: string
  bucket: string
  downloadPrefix: string
  uploadPrefix: string
}

export interface BuildEnvSpec {
  requiredInlineEnvKeys: readonly string[]
  inlineEnvKeys: readonly string[]
  ciInlineEnvDefaults?: Record<string, string>
  inlineEnvOverrides?: Record<string, string>
  defaultR2UploadPrefix: string
  defaultR2DownloadPrefix?: string
}

export interface BundleOptions {
  external?: readonly string[]
  plugins?: BunPlugin[]
}

export interface ProductBuildSpec {
  label: string
  packageDir: string
  env: BuildEnvSpec
}

export interface BuildProductDescriptor extends ProductBuildSpec {
  entrypoint: string
  distRoot: string
  rawBinaryBaseName: string
  stagedBinaryBaseName: string
  archiveBaseName: string
  defaultManifestPath: string
  defaultUpload?: boolean
  bundle?: BundleOptions
}

export interface AssetBuildProductDescriptor extends ProductBuildSpec {
  buildCommand: readonly string[]
  assetsDir: string
  distRoot: string
  archiveBaseName: string
  defaultUpload?: boolean
}

export interface AssetBuildArgs {
  upload: boolean
  ci: boolean
}

export interface StagedAssetArtifact {
  rootDir: string
  resourcesDir: string
  metadataPath: string
}

export interface BuildConfig {
  version: string
  envVars: Record<string, string>
  processEnv: NodeJS.ProcessEnv
  r2?: R2Config
}

export interface R2ResourceSource {
  type: 'r2'
  key: string
}

export interface LocalResourceSource {
  type: 'local'
  path: string
}

export type ResourceSource = R2ResourceSource | LocalResourceSource

export interface ResourceRule {
  name: string
  source: ResourceSource
  destination: string
  executable?: boolean
  recursive?: boolean
  os?: TargetOs[]
  arch?: TargetArch[]
}

export interface ResourceManifest {
  resources: ResourceRule[]
}

export interface CompiledServerBinary {
  target: BuildTarget
  binaryPath: string
}

export interface StagedArtifact {
  target: BuildTarget
  rootDir: string
  resourcesDir: string
  metadataPath: string
}

export interface UploadResult {
  targetId: TargetId
  zipPath: string
  latestR2Key?: string
  versionR2Key?: string
}
