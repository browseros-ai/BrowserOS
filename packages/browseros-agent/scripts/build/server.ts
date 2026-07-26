#!/usr/bin/env bun

import { runProdResourceBuild } from '@browseros/build-server-tools'

import { browserosServerBuildProduct } from './server/descriptor'

runProdResourceBuild(browserosServerBuildProduct, process.argv.slice(2)).catch(
  (error) => {
    const message = error instanceof Error ? error.message : String(error)
    console.error(`\n✗ ${message}\n`)
    process.exit(1)
  },
)
