# Third-Party Notices

`agent-mcp-manager` is authored under MIT. This file discloses the
third-party projects whose work informed the catalog and the
architectural choices behind v0.0.4.

No source code from any listed project is incorporated in the
published package. Where a project is listed under an AGPL license,
the disclosure exists because we read the project's public data as a
research reference; we did not import or copy code, and the published
package has no runtime dependency on it.

## smithery-ai/cli (design reference; AGPL-3.0)

- Source: https://github.com/smithery-ai/cli
- License: AGPL-3.0
- Role: research and cross-check reference for the per-client
  configuration data in `src/_catalog/client-configs.ts`. Every
  catalog entry has a `sources.smithery` URL pointing at the specific
  block we cross-checked in `src/config/clients.ts`. First-party MCP
  documentation for each client is the primary source
  (`sources.firstParty`); Smithery is corroboration only.
- What we DID: read the file to learn what fields exist per client,
  what tag values each client's parser expects, and which URL / env /
  command field names are non-default. Populated our own tables from
  a mix of first-party docs and this cross-check.
- What we did NOT do: copy the populated `CLIENTS` map or the
  interface bodies verbatim, depend on the Smithery npm package at
  runtime, or copy any command-based install templates.

## docker/mcp-gateway (historical reference; MIT)

- Source: https://github.com/docker/mcp-gateway
- License: MIT
- Role: the v0.0.3 catalog was hand-derived from
  `pkg/client/config.yml`. v0.0.4 replaces that with a hand-authored
  catalog whose write shapes are informed by both docker/mcp-gateway
  (for stdio) and smithery-ai/cli (for HTTP). The vendored YAML has
  been removed from this package as of v0.0.4; only the historical
  git history under the feat/mcp-manager-v0.0.4-fp branch retains it.
- No source code from mcp-gateway is incorporated in the published
  package.

### MIT License (docker/mcp-gateway)

```
Copyright (c) Docker, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```
