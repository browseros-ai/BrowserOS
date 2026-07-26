#!/usr/bin/env bun

import { runProdAssetBuild } from '@browseros/build-server-tools'

import { clawOnboardBuildProduct } from './claw-onboard/descriptor'

runProdAssetBuild(clawOnboardBuildProduct, process.argv.slice(2)).catch(
  (error) => {
    const message = error instanceof Error ? error.message : String(error)
    console.error(`\n✗ ${message}\n`)
    process.exit(1)
  },
)
