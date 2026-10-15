/** Describe the opt-in Rust artifact using the deployed BrowserOS server identity.
 * Cargo owns the version; the runtime basename and archive/R2 names stay compatible
 * with Chromium's existing download, launch, signing, and OTA paths.
 */
import type { ResourceBuildProductDescriptor } from '@browseros/build-server-tools'

export const serverRustBuildProduct: ResourceBuildProductDescriptor = {
  label: 'BrowserOS Rust server',
  packageDir: 'apps/server-rust',
  versionSource: {
    type: 'cargo-toml',
    path: 'apps/server-rust/Cargo.toml',
  },
  distRoot: 'dist/prod/server-rust',
  stagedBinaryBaseName: 'browseros_server',
  archiveBaseName: 'browseros-server-resources',
  defaultManifestPath: 'scripts/build/config/server-rust-prod-resources.json',
  // Publishing under the deployed identity is an explicit owner decision.
  defaultUpload: false,
  includeArtifactIdentity: true,
  archiveFilesOnly: true,
  expectedArtifactFiles: (target) => {
    const suffix = target.os === 'windows' ? '.exe' : ''
    return [
      `resources/bin/browseros_server${suffix}`,
      `resources/bin/third_party/bun${suffix}`,
    ]
  },
  env: {
    requiredInlineEnvKeys: ['POSTHOG_API_KEY', 'SENTRY_DSN'],
    inlineEnvKeys: ['POSTHOG_API_KEY', 'SENTRY_DSN', 'NODE_ENV', 'LOG_LEVEL'],
    defaultR2UploadPrefix: 'artifacts/server',
    defaultR2DownloadPrefix: 'artifacts/vendor',
  },
}
