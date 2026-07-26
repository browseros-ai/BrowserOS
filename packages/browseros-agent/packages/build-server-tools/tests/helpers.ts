import type {
  BuildProductDescriptor,
  BuildTarget,
  R2Config,
  TargetId,
} from '../src'

export function testProduct(
  overrides: Partial<BuildProductDescriptor> = {},
): BuildProductDescriptor {
  return {
    label: 'Test server',
    packageDir: 'apps/test-server',
    entrypoint: 'apps/test-server/src/main.ts',
    distRoot: 'dist/prod/test-server',
    rawBinaryBaseName: 'test-server',
    stagedBinaryBaseName: 'test_server',
    archiveBaseName: 'test-server-resources',
    defaultManifestPath: 'scripts/build/config/test-server-resources.json',
    env: {
      requiredInlineEnvKeys: ['TEST_CONFIG_URL'],
      inlineEnvKeys: ['TEST_CONFIG_URL', 'LOG_LEVEL'],
      ciInlineEnvDefaults: {
        LOG_LEVEL: 'info',
        TEST_CONFIG_URL: 'https://test.invalid/config',
      },
      defaultR2UploadPrefix: 'test-server/prod-resources',
      defaultR2DownloadPrefix: 'artifacts/vendor',
    },
    ...overrides,
  }
}

export function testTarget(
  id: TargetId = 'darwin-arm64',
  overrides: Partial<BuildTarget> = {},
): BuildTarget {
  const os = id.startsWith('windows')
    ? 'windows'
    : id.startsWith('darwin')
      ? 'macos'
      : 'linux'
  const arch = id.endsWith('arm64') ? 'arm64' : 'x64'
  return {
    id,
    name: id,
    os,
    arch,
    bunTarget: `bun-${id}`,
    ...overrides,
  }
}

export const fakeR2Config: R2Config = {
  accountId: 'test',
  accessKeyId: 'test',
  secretAccessKey: 'test',
  bucket: 'browseros-test',
  downloadPrefix: 'artifacts/vendor',
  uploadPrefix: 'server/prod-resources',
}
