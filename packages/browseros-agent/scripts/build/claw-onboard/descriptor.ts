import type { AssetBuildProductDescriptor } from '@browseros/build-server-tools'

const INLINED_ENV_VARS = ['NODE_ENV'] as const
const PRODUCTION_INLINE_ENV = {
  NODE_ENV: 'production',
}

// build:chromium runs tsc, vite --mode chromium, and the WebUI contract
// verification, so the staged dist is always contract-checked before upload.
export const clawOnboardBuildProduct: AssetBuildProductDescriptor = {
  label: 'BrowserOS Claw onboarding',
  packageDir: 'apps/claw-onboard',
  buildCommand: ['bun', 'run', 'build:chromium'],
  assetsDir: 'apps/claw-onboard/dist/chromium',
  distRoot: 'dist/prod/claw-onboard',
  archiveBaseName: 'browseros-claw-onboard-resources',
  env: {
    requiredInlineEnvKeys: [],
    inlineEnvKeys: INLINED_ENV_VARS,
    ciInlineEnvDefaults: PRODUCTION_INLINE_ENV,
    inlineEnvOverrides: PRODUCTION_INLINE_ENV,
    defaultR2UploadPrefix: 'claw-onboard/prod-resources',
  },
}
