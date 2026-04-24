/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export const VM_TELEMETRY_EVENTS = {
  ensureReadyStart: 'vm.ensure_ready.start',
  ensureReadyOk: 'vm.ensure_ready.ok',
  ensureReadyBranch: 'vm.ensure_ready.branch',
  create: 'vm.create',
  start: 'vm.start',
  stop: 'vm.stop',
  upgradeDetected: 'vm.upgrade.detected',
  downgradeDetected: 'vm.downgrade.detected',
  upgradeSwap: 'vm.upgrade.swap',
  upgradeReplay: 'vm.upgrade.replay',
  resetDetected: 'vm.reset.detected',
  resetOk: 'vm.reset.ok',
  nerdctlWaitStart: 'vm.nerdctl_wait.start',
  nerdctlWaitOk: 'vm.nerdctl_wait.ok',
  nerdctlWaitPoll: 'vm.nerdctl_wait.poll',
  nerdctlWaitTimeout: 'vm.nerdctl_wait.timeout',
  manifestMissing: 'vm.manifest.missing',
  manifestCompared: 'vm.manifest.compared',
  manifestWritten: 'vm.manifest.written',
  migrationOpenClawMoved: 'vm.migration.openclaw_moved',
  limaSpawn: 'vm.lima.spawn',
  limaExit: 'vm.lima.exit',
  limaStderrChunk: 'vm.lima.stderr_chunk',
  provisionYamlWrite: 'vm.provision.yaml_write',
  provisionCreateStart: 'vm.provision.create.start',
  provisionCreateOk: 'vm.provision.create.ok',
  provisionStartBegin: 'vm.provision.start.begin',
  provisionStartOk: 'vm.provision.start.ok',
} as const
