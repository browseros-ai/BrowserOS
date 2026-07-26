import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'

const buildDir = path.resolve(import.meta.dir, '../dist/chromium')
const textResourceFiles = ['app.css', 'app.js', 'index.html']
const iconResourceFiles = [
  'icon/16.png',
  'icon/32.png',
  'icon/48.png',
  'icon/96.png',
  'icon/128.png',
]
const allowedFiles = new Set([...textResourceFiles, ...iconResourceFiles])
const fontAssetPattern = /\.(?:woff2?|ttf|otf)$/i
const hashedCoreAssetPattern = /\b(?:app|index)-[A-Za-z0-9_-]{6,}\.(?:css|js)\b/
const dataUrlPattern = /\bdata:(?:[a-z][\w.+-]*\/[a-z0-9.+-]+|;base64|,)/i
const remoteUrlPattern = /https?:\/\/[^\s"'<>\\)]+/g
const expectedRuntimeUrlPatterns = [
  /^http:\/\/127\.0\.0\.1:(?:\d+|\$\{[$A-Z_a-z][\w$]*\})(?:$|\/mcp(?:$|[^\w./:?#-])|[^\w./:?#-])/,
  /^http:\/\/localhost(?::\d+)?(?:$|\/mcp(?:$|[^\w./:?#-])|[^\w./:?#-])/,
  /^http:\/\/www\.w3\.org\/(?:1998\/Math\/MathML|1999\/xlink|2000\/svg|XML\/1998\/namespace)$/,
  /^http:\/\/json-schema\.org\/draft-(?:04|07)\/schema#$/,
  /^https:\/\/base-ui\.com\/production-error$/,
  /^https:\/\/json-schema\.org\/draft\/2020-12\/schema$/,
  /^https:\/\/react\.dev\/errors\/(?:[A-Za-z0-9_-]+)?$/,
  /^https:\/\/reactrouter\.com\/en\/main\/routers\/picking-a-router\.(?:$|[^\w./:-])/,
]

function fail(message: string): never {
  throw new Error(`Chromium build verification failed: ${message}`)
}

async function listResourceFiles(dir: string, prefix = ''): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true })
  const files: string[] = []

  for (const entry of entries) {
    const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name
    if (entry.isDirectory()) {
      if (relativePath === 'assets') fail('dist/chromium/assets was emitted')
      if (relativePath !== 'icon')
        fail(`unexpected directory emitted: ${relativePath}`)
      files.push(
        ...(await listResourceFiles(path.join(dir, entry.name), relativePath)),
      )
      continue
    }
    files.push(relativePath)
  }

  return files.sort()
}

function verifyFileList(files: string[]) {
  for (const expected of allowedFiles) {
    if (!files.includes(expected)) fail(`missing ${expected}`)
  }

  for (const file of files) {
    if (!allowedFiles.has(file)) fail(`unexpected file emitted: ${file}`)
    if (fontAssetPattern.test(file)) fail(`font asset emitted: ${file}`)
    if (hashedCoreAssetPattern.test(file))
      fail(`hashed core resource emitted: ${file}`)
  }
}

async function readResource(file: string): Promise<string> {
  return readFile(path.join(buildDir, file), 'utf8')
}

function isExpectedRuntimeUrl(url: string): boolean {
  return expectedRuntimeUrlPatterns.some((pattern) => pattern.test(url))
}

function verifyIndexReferences(indexHtml: string) {
  const hasCss =
    /<link\b(?=[^>]*\brel=["']stylesheet["'])(?=[^>]*\bhref=["']\.\/app\.css["'])[^>]*>/i.test(
      indexHtml,
    )
  const hasJs = /<script\b(?=[^>]*\bsrc=["']\.\/app\.js["'])[^>]*>/i.test(
    indexHtml,
  )

  if (!hasCss) fail('index.html does not reference ./app.css')
  if (!hasJs) fail('index.html does not reference ./app.js')
  if (!indexHtml.includes('href="./icon/32.png"')) {
    fail('index.html does not reference ./icon/32.png')
  }
}

function verifyResourceContents(file: string, contents: string) {
  if (dataUrlPattern.test(contents)) fail(`${file} contains a data: URL`)
  if (contents.includes('assets/')) fail(`${file} references assets/`)
  if (hashedCoreAssetPattern.test(contents)) {
    fail(`${file} references a hashed core resource`)
  }

  const remoteUrls = contents.match(remoteUrlPattern) ?? []
  const unexpectedUrls = remoteUrls.filter((url) => !isExpectedRuntimeUrl(url))
  if (unexpectedUrls.length > 0) {
    fail(`${file} references remote URLs: ${unexpectedUrls.join(', ')}`)
  }
}

/** Enforces the Chromium WebUI resource contract, including favicon assets. */
async function main() {
  const files = await listResourceFiles(buildDir)
  verifyFileList(files)

  const indexHtml = await readResource('index.html')
  verifyIndexReferences(indexHtml)

  for (const file of textResourceFiles) {
    verifyResourceContents(file, await readResource(file))
  }
}

await main()
