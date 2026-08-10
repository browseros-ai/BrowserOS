import type { ResourceBuildProductDescriptor } from '@browseros/build-server-tools'

const INLINE_ENV_KEYS = ['CLAW_POSTHOG_KEY', 'CLAW_POSTHOG_HOST'] as const

export const clawServerRustBuildProduct: ResourceBuildProductDescriptor = {
  label: 'BrowserClaw Rust server',
  packageDir: 'apps/claw-server-rust',
  versionSource: {
    type: 'cargo-toml',
    path: 'apps/claw-server-rust/Cargo.toml',
  },
  distRoot: 'dist/prod/claw-server-rust',
  stagedBinaryBaseName: 'browseros-claw-server',
  archiveBaseName: 'browseros-claw-server-rust-resources',
  defaultManifestPath:
    'scripts/build/config/claw-server-rust-prod-resources.json',
  includeArtifactIdentity: true,
  archiveFilesOnly: true,
  expectedArtifactFiles: (target) => [
    `resources/bin/browseros-claw-server${target.os === 'windows' ? '.exe' : ''}`,
    'resources/skills/browserclaw/SKILL.md',
  ],
  env: {
    requiredInlineEnvKeys: ['CLAW_POSTHOG_KEY'],
    inlineEnvKeys: INLINE_ENV_KEYS,
    ciInlineEnvOverrides: {
      CLAW_POSTHOG_KEY: 'phc_browseros_ci',
    },
    defaultR2UploadPrefix: 'claw-server-rust/prod-resources',
  },
}
