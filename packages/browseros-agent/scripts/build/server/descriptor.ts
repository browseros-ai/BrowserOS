import type { ResourceBuildProductDescriptor } from '@browseros/build-server-tools'

const packageDir = 'crates/browseros-server'

export const browserosServerBinary = {
  packageName: 'browseros-server',
  binaryName: 'browseros_server',
}

/**
 * Cargo settings every BrowserOS server build applies, local or CI. Rust links
 * the MSVC C runtime dynamically by default, which makes the server need
 * VCRUNTIME140.dll from the Visual C++ redistributable, absent on a clean
 * Windows install. A static CRT keeps the exe on system DLLs only, as the old
 * Bun build was. The variable is target-scoped, so other targets are unaffected.
 * `.github/workflows/release-server.yml` and bos_build's BrowserOS bundle set
 * the same value, and a test holds the workflow to this one.
 */
export const browserosServerCargoEnv = {
  CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS:
    '-C target-feature=+crt-static',
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
