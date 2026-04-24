/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
import { OPENCLAW_GATEWAY_CONTAINER_NAME } from '@browseros/shared/constants/openclaw'
import { ContainerRuntime } from '../../../../src/api/services/openclaw/container-runtime'

const PROJECT_DIR = '/tmp/openclaw'
const defaultSpec = {
  image: 'ghcr.io/openclaw/openclaw:2026.4.12',
  hostPort: 18789,
  hostHome: '/Users/me/.browseros/vm/openclaw',
  envFilePath: '/Users/me/.browseros/vm/openclaw/.openclaw/.env',
  gatewayToken: 'token-123',
  timezone: 'America/Los_Angeles',
}

describe('ContainerRuntime', () => {
  it('starts the gateway by loading the image, creating, and starting a container', async () => {
    const deps = createDeps()
    const runtime = new ContainerRuntime({
      vm: deps.vm,
      shell: deps.shell,
      loader: deps.loader,
      projectDir: PROJECT_DIR,
    })

    await runtime.startGateway(defaultSpec)

    expect(deps.shell.removeContainer).toHaveBeenCalledWith(
      OPENCLAW_GATEWAY_CONTAINER_NAME,
      { force: true },
      undefined,
    )
    expect(deps.loader.ensureImageLoaded).toHaveBeenCalledWith(
      defaultSpec.image,
      undefined,
    )
    expect(deps.shell.createContainer).toHaveBeenCalledWith(
      expect.objectContaining({
        name: OPENCLAW_GATEWAY_CONTAINER_NAME,
        image: defaultSpec.image,
        restart: 'unless-stopped',
        ports: [
          {
            hostIp: '127.0.0.1',
            hostPort: 18789,
            containerPort: 18789,
          },
        ],
        envFile: '/mnt/browseros/vm/openclaw/.openclaw/.env',
        mounts: [
          {
            source: '/mnt/browseros/vm/openclaw',
            target: '/home/node',
          },
        ],
        addHosts: ['host.containers.internal:192.168.5.2'],
      }),
      undefined,
    )
    expect(deps.shell.startContainer).toHaveBeenCalledWith(
      OPENCLAW_GATEWAY_CONTAINER_NAME,
    )
  })

  it('delegates ensureReady and stopVm to VmRuntime', async () => {
    const deps = createDeps()
    const runtime = new ContainerRuntime({
      vm: deps.vm,
      shell: deps.shell,
      loader: deps.loader,
      projectDir: PROJECT_DIR,
    })

    await runtime.ensureReady()
    await runtime.stopVm()

    expect(deps.vm.ensureReady).toHaveBeenCalled()
    expect(deps.vm.getDefaultGateway).toHaveBeenCalled()
    expect(deps.vm.stopVm).toHaveBeenCalled()
  })

  it('runs setup commands with guest paths', async () => {
    const deps = createDeps()
    const runtime = new ContainerRuntime({
      vm: deps.vm,
      shell: deps.shell,
      loader: deps.loader,
      projectDir: PROJECT_DIR,
    })

    await runtime.runGatewaySetupCommand(
      ['node', 'dist/index.js', 'agents', 'list', '--json'],
      defaultSpec,
    )

    expect(deps.shell.runCommand).toHaveBeenCalledWith(
      expect.arrayContaining([
        'create',
        '--name',
        `${OPENCLAW_GATEWAY_CONTAINER_NAME}-setup`,
        '--env-file',
        '/mnt/browseros/vm/openclaw/.openclaw/.env',
        '-v',
        '/mnt/browseros/vm/openclaw:/home/node',
        '--add-host',
        'host.containers.internal:192.168.5.2',
      ]),
      undefined,
    )
    expect(deps.shell.runCommand).toHaveBeenCalledWith(
      ['start', '-a', `${OPENCLAW_GATEWAY_CONTAINER_NAME}-setup`],
      undefined,
    )
    expect(deps.shell.removeContainer).toHaveBeenCalledWith(
      `${OPENCLAW_GATEWAY_CONTAINER_NAME}-setup`,
      { force: true },
      undefined,
    )
  })

  it('tails and fetches gateway logs through the new transport', async () => {
    const deps = createDeps()
    const runtime = new ContainerRuntime({
      vm: deps.vm,
      shell: deps.shell,
      loader: deps.loader,
      projectDir: PROJECT_DIR,
    })

    const stop = runtime.tailGatewayLogs(() => {})
    const logs = await runtime.getGatewayLogs(10)
    stop()

    expect(deps.shell.tailLogs).toHaveBeenCalledWith(
      OPENCLAW_GATEWAY_CONTAINER_NAME,
      expect.any(Function),
    )
    expect(deps.shell.runCommand).toHaveBeenCalledWith(
      ['logs', '-n', '10', OPENCLAW_GATEWAY_CONTAINER_NAME],
      expect.any(Function),
    )
    expect(logs).toEqual(['log line'])
  })
})

function createDeps() {
  return {
    vm: {
      ensureReady: mock(async () => {}),
      getDefaultGateway: mock(async () => '192.168.5.2'),
      stopVm: mock(async () => {}),
      isReady: mock(async () => true),
    },
    shell: {
      createContainer: mock(async () => {}),
      startContainer: mock(async () => {}),
      stopContainer: mock(async () => {}),
      removeContainer: mock(async () => {}),
      exec: mock(async () => 0),
      runCommand: mock(
        async (_args: string[], onLog?: (line: string) => void) => {
          onLog?.('log line')
          return { exitCode: 0, stdout: 'log line\n', stderr: '' }
        },
      ),
      tailLogs: mock(() => () => {}),
    },
    loader: {
      ensureImageLoaded: mock(async () => {}),
    },
  }
}
