import type { ResourceBuildProductDescriptor } from '@browseros/build-server-tools'

const packageDir = 'crates/browseros-server'

export const browserosServerBinary = {
  packageName: 'browseros-server',
  binaryName: 'browseros_server',
}

export const browserosServerBuildProduct: ResourceBuildProductDescriptor = {
  label: 'BrowserOS Rust server',
  packageDir,
  versionSource: {
    type: 'cargo-toml',
    path: `${packageDir}/Cargo.toml`,
  },
  // The browser and release tooling use these names to find the bundled server.
  distRoot: 'dist/prod/server',
  stagedBinaryBaseName: 'browseros_server',
  archiveBaseName: 'browseros-server-resources',
  defaultManifestPath: 'scripts/build/config/server-prod-resources.json',
  env: {
    requiredInlineEnvKeys: [],
    inlineEnvKeys: ['BROWSEROS_SENTRY_DSN'],
    defaultR2UploadPrefix: 'artifacts/server',
    defaultR2DownloadPrefix: 'artifacts/vendor',
  },
}
