import { describe, expect, it } from 'bun:test'
import { readFile } from 'node:fs/promises'
import path from 'node:path'

const templatePath = path.resolve(
  import.meta.dir,
  '../template/browseros-vm.yaml',
)

describe('browseros-vm Lima template', () => {
  it('uses Ubuntu minimal with Lima-managed rootless containerd and nerdctl', async () => {
    const yaml = await readFile(templatePath, 'utf8')

    expect(yaml).toContain('ubuntu-24.04-minimal-cloudimg-arm64.img')
    expect(yaml).toContain('ubuntu-24.04-minimal-cloudimg-amd64.img')
    expect(yaml).toContain('containerd:')
    expect(yaml).toContain('system: false')
    expect(yaml).toContain('user: true')
    expect(yaml).toContain('until nerdctl info >/dev/null 2>&1')
    expect(yaml).toContain('runtime:containerd-rootless')
    expect(yaml).toContain(
      'guestSocket: "/run/user/{{.UID}}/containerd-rootless/containerd.sock"',
    )
    expect(yaml).toContain('hostSocket: "{{.Dir}}/sock/containerd.sock"')
    expect(yaml).not.toContain('sudo nerdctl')
    expect(yaml).not.toContain('/var/run/containerd/containerd.sock')
    expect(yaml).not.toContain('podman')
    expect(yaml).not.toContain('debian')
  })
})
