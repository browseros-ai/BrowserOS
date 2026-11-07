/** Reproduces the upstream TestServer reset requirement without changing the upstream server. */
import http from 'node:http'

const server = http.createServer()
await new Promise((resolve) => server.listen(0, resolve))
const before = server.address()
server.closeAllConnections()
const after = server.address()
await new Promise((resolve) => server.close(resolve))
console.log(
  JSON.stringify({
    host: process.versions.bun ? 'bun' : 'node',
    version: process.versions.bun || process.versions.node,
    upstreamResetPreservesListeningSocket: Boolean(before && after),
    before,
    after,
  }),
)
