/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * OpenClaw CLI-backed provider registry types.
 *
 * A "CLI provider" is a tool that runs inside the OpenClaw gateway
 * container (e.g. Claude Code CLI, Gemini CLI). OpenClaw spawns the
 * binary as a subprocess when the active model is prefixed with the
 * provider id — so our job is to install the tool and surface its
 * auth status to the user. No Anthropic/OpenRouter-style API key.
 */

export interface OpenClawCliProviderAuthStatus {
  installed: boolean
  loggedIn: boolean
  accountLabel?: string
  subscriptionLabel?: string
  error?: string
}

export interface OpenClawCliProvider {
  id: string
  displayName: string
  description: string
  npmPackage: string
  // Pinned package version. npm installs go through argv directly
  // (no shell), so `@latest` drift can't silently ship through.
  npmPackageVersion: string
  binary: string
  authStatusCommand: string[]
  authLoginCommand: string
  models: readonly string[]
  parseAuthStatus: (
    stdout: string,
    exitCode: number,
  ) => OpenClawCliProviderAuthStatus
}
