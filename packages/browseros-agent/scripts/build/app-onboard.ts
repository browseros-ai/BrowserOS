#!/usr/bin/env bun

import { runProdAssetBuild } from '@browseros/build-server-tools'

import { appOnboardBuildProduct } from './app-onboard/descriptor'

runProdAssetBuild(appOnboardBuildProduct, process.argv.slice(2)).catch(
  (error) => {
    const message = error instanceof Error ? error.message : String(error)
    console.error(`\n✗ ${message}\n`)
    process.exit(1)
  },
)
