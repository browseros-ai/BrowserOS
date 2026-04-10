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
  serverBinaryName: string
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

export interface BuildConfig {
  version: string
  envVars: Record<string, string>
  processEnv: NodeJS.ProcessEnv
  r2?: R2Config
}

export interface ResourceSource {
  type: 'r2'
  key: string
}

export interface ResourceRule {
  name: string
  source: ResourceSource
  destination: string
  executable?: boolean
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
