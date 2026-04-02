const https = require('node:https')
const http = require('node:http')
const fs = require('node:fs')
const path = require('node:path')
const { execSync } = require('node:child_process')
const { createHash } = require('node:crypto')

const VERSION = require('../package.json').version
const GITHUB_RELEASE_BASE = `https://github.com/browseros-ai/BrowserOS/releases/download/browseros-cli-v${VERSION}`
const BINARY_DIR = path.join(__dirname, '..', '.binary')
const EXT = process.platform === 'win32' ? '.exe' : ''
const BINARY_PATH = path.join(BINARY_DIR, `browseros-cli${EXT}`)

if (process.env.CI && !process.env.BROWSEROS_NPM_FORCE) {
  process.exit(0)
}

const PLATFORM_MAP = { darwin: 'darwin', linux: 'linux', win32: 'windows' }
const ARCH_MAP = { x64: 'amd64', arm64: 'arm64' }

const platform = PLATFORM_MAP[process.platform]
const arch = ARCH_MAP[process.arch]

if (!platform || !arch) {
  console.error(
    `browseros-cli: unsupported platform ${process.platform}/${process.arch}`,
  )
  process.exit(1)
}

const isWindows = platform === 'windows'
const archiveExt = isWindows ? 'zip' : 'tar.gz'
const archiveName = `browseros-cli_${VERSION}_${platform}_${arch}.${archiveExt}`
const archiveURL = `${GITHUB_RELEASE_BASE}/${archiveName}`
const checksumURL = `${GITHUB_RELEASE_BASE}/checksums.txt`

const MAX_REDIRECTS = 5

function download(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    if (redirects > MAX_REDIRECTS) {
      return reject(new Error(`Too many redirects for ${url}`))
    }
    const client = url.startsWith('https') ? https : http
    client
      .get(url, { headers: { 'User-Agent': 'browseros-cli-npm' } }, (res) => {
        if (
          res.statusCode >= 300 &&
          res.statusCode < 400 &&
          res.headers.location
        ) {
          return download(res.headers.location, redirects + 1).then(
            resolve,
            reject,
          )
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`))
        }
        const chunks = []
        res.on('data', (chunk) => chunks.push(chunk))
        res.on('end', () => resolve(Buffer.concat(chunks)))
        res.on('error', reject)
      })
      .on('error', reject)
  })
}

async function main() {
  console.log(
    `browseros-cli: downloading v${VERSION} for ${platform}/${arch}...`,
  )

  const [archiveBuffer, checksumBuffer] = await Promise.all([
    download(archiveURL),
    download(checksumURL).catch(() => null),
  ])

  if (checksumBuffer) {
    const checksumText = checksumBuffer.toString('utf-8')
    const expectedLine = checksumText
      .split('\n')
      .find((l) => l.includes(archiveName))
    if (expectedLine) {
      const expected = expectedLine.split(/\s+/)[0]
      const actual = createHash('sha256').update(archiveBuffer).digest('hex')
      if (actual !== expected) {
        console.error(
          `browseros-cli: checksum mismatch!\n  expected: ${expected}\n  got:      ${actual}`,
        )
        process.exit(1)
      }
      console.log('browseros-cli: checksum verified.')
    } else {
      console.warn(
        'browseros-cli: warning: checksum entry not found in checksums.txt, skipping verification.',
      )
    }
  } else {
    console.warn(
      'browseros-cli: warning: could not fetch checksums.txt, skipping verification.',
    )
  }

  fs.mkdirSync(BINARY_DIR, { recursive: true })
  const tmpArchive = path.join(BINARY_DIR, archiveName)
  fs.writeFileSync(tmpArchive, archiveBuffer)

  if (isWindows) {
    execSync(
      `powershell -Command "Expand-Archive -Force -Path '${tmpArchive}' -DestinationPath '${BINARY_DIR}'"`,
      { stdio: 'inherit' },
    )
  } else {
    execSync(`tar -xzf "${tmpArchive}" -C "${BINARY_DIR}"`, {
      stdio: 'inherit',
    })
  }

  fs.unlinkSync(tmpArchive)

  if (!fs.existsSync(BINARY_PATH)) {
    console.error(
      `browseros-cli: binary not found after extraction at ${BINARY_PATH}`,
    )
    process.exit(1)
  }

  if (!isWindows) {
    fs.chmodSync(BINARY_PATH, 0o755)
  }

  console.log(`browseros-cli: installed v${VERSION} successfully.`)
}

main().catch((err) => {
  console.error(`browseros-cli: installation failed: ${err.message}`)
  console.error(
    'You can install manually: curl -fsSL https://cdn.browseros.com/cli/install.sh | bash',
  )
  process.exit(1)
})
