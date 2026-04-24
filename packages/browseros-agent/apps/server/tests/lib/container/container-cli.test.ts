/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { ContainerCli } from '../../../src/lib/container/container-cli'
import { ContainerCliError } from '../../../src/lib/vm/errors'
import { fakeSsh } from '../../__helpers__/fake-ssh'

describe('ContainerCli', () => {
  let tempDir: string
  let logPath: string

  beforeEach(async () => {
    tempDir = await mkdtemp('/tmp/container-cli-')
    logPath = join(tempDir, 'ssh.log')
  })

  afterEach(async () => {
    await rm(tempDir, { recursive: true, force: true })
  })

  it('checks image existence with nerdctl image inspect', async () => {
    const sshPath = await fakeSsh({}, logPath)
    const cli = await createCli(sshPath, tempDir)

    await expect(cli.imageExists('openclaw:v1')).resolves.toBe(true)

    const sshConfig = sshConfigPath(tempDir)
    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      `${sshPrefix(sshConfig)} 'nerdctl' 'image' 'inspect' 'openclaw:v1'`,
    )
  })

  it('returns false when image inspect exits non-zero', async () => {
    const sshPath = await fakeSsh({ stderr: 'missing', exit: 1 }, logPath)
    const cli = await createCli(sshPath, tempDir)

    await expect(cli.imageExists('openclaw:v1')).resolves.toBe(false)
  })

  it('pulls images with progress and throws typed command errors', async () => {
    const sshPath = await fakeSsh(
      { stdout: 'pulling\n', stderr: 'denied', exit: 2 },
      logPath,
    )
    const cli = await createCli(sshPath, tempDir)
    const lines: string[] = []

    const error = await cli
      .pullImage('openclaw:v1', (line) => lines.push(line))
      .catch((err) => err)

    expect(error).toBeInstanceOf(ContainerCliError)
    expect(error.exitCode).toBe(2)
    expect(error.stderr).toBe('denied')
    expect(lines).toContain('pulling')
    expect(lines).toContain('denied')
  })

  it('loads images from guest tarballs and returns loaded refs', async () => {
    const sshPath = await fakeSsh(
      { stdout: 'Loaded image(s): openclaw:v1\n' },
      logPath,
    )
    const cli = await createCli(sshPath, tempDir)

    await expect(
      cli.loadImage('/mnt/browseros/cache/images/openclaw.tar.gz'),
    ).resolves.toEqual(['openclaw:v1'])
    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      `${sshPrefix(sshConfigPath(tempDir))} 'nerdctl' 'load' '-i' '/mnt/browseros/cache/images/openclaw.tar.gz'`,
    )
  })

  it('creates containers from typed specs', async () => {
    const sshPath = await fakeSsh({}, logPath)
    const cli = await createCli(sshPath, tempDir)

    await cli.createContainer({
      name: 'gateway',
      image: 'openclaw:v1',
      restart: 'unless-stopped',
      ports: [{ hostIp: '127.0.0.1', hostPort: 18789, containerPort: 18789 }],
      envFile: '/mnt/browseros/vm/openclaw/.env',
      env: { HOME: '/home/node', NODE_ENV: 'production' },
      mounts: [
        {
          source: '/mnt/browseros/vm/openclaw',
          target: '/home/node',
          readonly: true,
        },
      ],
      addHosts: ['host.containers.internal:192.168.5.2'],
      health: {
        cmd: 'curl -sf http://127.0.0.1:18789/healthz',
        interval: '30s',
        timeout: '10s',
        retries: 3,
      },
      command: ['node', 'dist/index.js', 'gateway'],
    })

    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      [
        `${sshPrefix(sshConfigPath(tempDir))} 'nerdctl' 'create'`,
        "'--name' 'gateway'",
        "'--restart' 'unless-stopped'",
        "'-p' '127.0.0.1:18789:18789'",
        "'--env-file' '/mnt/browseros/vm/openclaw/.env'",
        "'-e' 'HOME=/home/node'",
        "'-e' 'NODE_ENV=production'",
        "'-v' '/mnt/browseros/vm/openclaw:/home/node:ro'",
        "'--add-host' 'host.containers.internal:192.168.5.2'",
        "'--health-cmd' 'curl -sf http://127.0.0.1:18789/healthz'",
        "'--health-interval' '30s'",
        "'--health-timeout' '10s'",
        "'--health-retries' '3'",
        "'openclaw:v1' 'node' 'dist/index.js' 'gateway'",
      ].join(' '),
    )
  })

  it('starts, stops, removes, execs, and lists containers', async () => {
    const sshPath = await fakeSsh({ stdout: 'gateway\nworker\n' }, logPath)
    const cli = await createCli(sshPath, tempDir)

    await cli.startContainer('gateway')
    await cli.stopContainer('gateway')
    await cli.removeContainer('gateway', { force: true })
    await expect(cli.exec('gateway', ['node', '--version'])).resolves.toBe(0)
    await expect(cli.ps({ namesOnly: true })).resolves.toEqual([
      'gateway',
      'worker',
    ])

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain("lima-browseros-vm 'nerdctl' 'start' 'gateway'")
    expect(log).toContain("lima-browseros-vm 'nerdctl' 'stop' 'gateway'")
    expect(log).toContain("lima-browseros-vm 'nerdctl' 'rm' '-f' 'gateway'")
    expect(log).toContain(
      "lima-browseros-vm 'nerdctl' 'exec' 'gateway' 'node' '--version'",
    )
    expect(log).toContain(
      "lima-browseros-vm 'nerdctl' 'ps' '--format' '{{.Names}}'",
    )
  })

  it('tolerates removal when the container is already absent', async () => {
    const sshPath = await fakeSsh(
      { stderr: 'no such container', exit: 1 },
      logPath,
    )
    const cli = await createCli(sshPath, tempDir)

    await expect(cli.removeContainer('gateway', { force: true })).resolves.toBe(
      undefined,
    )
  })

  it('tails logs and returns a stop handle', async () => {
    const sshPath = await fakeSsh({ stdout: 'line\n' }, logPath)
    const cli = await createCli(sshPath, tempDir)
    const lines: string[] = []

    const stop = cli.tailLogs('gateway', (line) => lines.push(line))
    await Bun.sleep(20)
    stop()

    expect(lines).toEqual(['line'])
    await expect(readFile(logPath, 'utf8')).resolves.toContain(
      `${sshPrefix(sshConfigPath(tempDir))} 'nerdctl' 'logs' '-f' '-n' '0' 'gateway'`,
    )
  })
})

async function createCli(
  sshPath: string,
  tempDir: string,
): Promise<ContainerCli> {
  const configPath = sshConfigPath(tempDir)
  await mkdir(join(tempDir, 'lima', 'browseros-vm'), { recursive: true })
  await writeFile(configPath, '')
  return new ContainerCli({
    limactlPath: 'unused',
    limaHome: join(tempDir, 'lima'),
    sshPath,
    vmName: 'browseros-vm',
  })
}

function sshConfigPath(tempDir: string): string {
  return join(tempDir, 'lima', 'browseros-vm', 'ssh.config')
}

function sshPrefix(configPath: string): string {
  return `ARGS:-F ${configPath} lima-browseros-vm`
}
