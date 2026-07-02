import type { BuildProductDescriptor } from '@browseros/build-server-tools'

export const CLAW_SERVER_BUNDLE_ENTRYPOINT = 'apps/claw-server/src/main.ts'

export const clawServerBuildProduct: BuildProductDescriptor = {
  label: 'BrowserOS Claw server',
  packageDir: 'apps/claw-server',
  entrypoint: CLAW_SERVER_BUNDLE_ENTRYPOINT,
  distRoot: 'dist/prod/claw-server',
  rawBinaryBaseName: 'browseros-claw-server',
  stagedBinaryBaseName: 'browseros-claw-server',
  archiveBaseName: 'browseros-claw-server-resources',
  defaultManifestPath: 'scripts/build/config/claw-server-prod-resources.json',
  env: {
    prodEnvPath: 'apps/claw-server/.env.production',
    requireProdEnvFile: false,
    requiredInlineEnvKeys: [],
    inlineEnvKeys: [],
    ciInlineEnvDefaults: {},
    defaultR2UploadPrefix: 'claw-server/prod-resources',
  },
}
