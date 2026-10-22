import type { ResourceBuildProductDescriptor } from '@browseros/build-server-tools'

// `CLAW_SENTRY_DSN` is inlined but deliberately not required: a build without it still
// works, and the server writes its redacted run-failure reports to a local log instead.
const INLINE_ENV_KEYS = [
  'CLAW_POSTHOG_KEY',
  'CLAW_POSTHOG_HOST',
  'CLAW_SENTRY_DSN',
] as const

export const clawServerBuildProduct: ResourceBuildProductDescriptor = {
  label: 'BrowserClaw Rust server',
  packageDir: 'apps/claw-server',
  versionSource: {
    type: 'cargo-toml',
    path: 'apps/claw-server/Cargo.toml',
  },
  distRoot: 'dist/prod/claw-server',
  stagedBinaryBaseName: 'browseros-claw-server',
  // Published archives and R2 keys keep their existing identity so browser builds
  // can still download earlier releases after the source package rename.
  archiveBaseName: 'browseros-claw-server-rust-resources',
  defaultManifestPath: 'scripts/build/config/claw-server-prod-resources.json',
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
