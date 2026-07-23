# @browseros/cdp-protocol

Type-safe Chrome DevTools Protocol bindings for BrowserOS.

> **Internal package** — auto-generated TypeScript types and API wrappers for all CDP domains. Used by `@browseros/server` to communicate with Chromium.

## Usage

Import domain types or domain API wrappers using subpath exports:

```typescript
// Import type definitions for a CDP domain
import type { NavigateParams, NavigateReturn } from '@browseros/cdp-protocol/domains/page'

// Import the API wrapper for a domain
import { PageAPI } from '@browseros/cdp-protocol/domain-apis/page'

// Core protocol API
import { ProtocolAPI } from '@browseros/cdp-protocol/protocol-api'

// Factory function
import { createAPI } from '@browseros/cdp-protocol/create-api'
```

## Supported Domains

All standard Chrome DevTools Protocol domains are supported:

| Category | Domains |
|----------|---------|
| **Page & DOM** | Page, DOM, DOMDebugger, DOMSnapshot, DOMStorage, CSS, Overlay |
| **Network** | Network, Fetch, IO, ServiceWorker, CacheStorage |
| **Input & Interaction** | Input, Emulation, DeviceOrientation, DeviceAccess |
| **JavaScript** | Runtime, Debugger, Console, Profiler, HeapProfiler |
| **Browser** | Browser, Target, Inspector, Extensions, PWA |
| **Performance** | Performance, PerformanceTimeline, Tracing, Memory |
| **Media** | Media, WebAudio, Cast |
| **Security** | Security, WebAuthn, FedCm |
| **Storage** | IndexedDB, Storage, FileSystem |
| **Other** | Accessibility, Animation, Audits, Autofill, BackgroundService, BluetoothEmulation, EventBreakpoints, HeadlessExperimental, LayerTree, Log, Preload, Schema, SystemInfo, Tethering |
| **BrowserOS Custom** | Bookmarks, CrashReportContext, History, WebMCP |

## Structure

```
src/generated/
├── domains/            # Type definitions for each CDP domain
│   ├── page.ts
│   ├── dom.ts
│   ├── network.ts
│   └── ...
├── domain-apis/        # API wrapper classes for each domain
│   ├── page.ts
│   ├── dom.ts
│   ├── network.ts
│   └── ...
├── protocol-api.ts     # Unified protocol API
└── create-api.ts       # API factory
```

## Regenerating Types

Types are auto-generated from the CDP protocol specification. The generated output lives in `src/generated/` and should not be edited manually.

1. Build Chromium so the generated DevTools protocol JSON exists:

   ```sh
   autoninja -C out/Default_arm64 chrome
   ```

2. From this repository root, point `CDP_PROTOCOL_JSON` at Chromium's generated protocol file:

   ```sh
   CDP_PROTOCOL_JSON=/path/to/chromium/src/out/Default_arm64/gen/third_party/blink/public/devtools_protocol/protocol.json bun run gen:cdp
   ```

   You can also set `CDP_PROTOCOL_JSON` in the monorepo root `.env.development`; `bun run gen:cdp` loads it with `--env-file`.

3. Review and commit all regenerated files:

   ```sh
   git status --short packages/cdp-protocol package.json scripts/codegen
   ```

   New CDP domains create new files under both `src/generated/domains/` and `src/generated/domain-apis/`. Make sure those files are tracked along with `packages/cdp-protocol/package.json`, `protocol-api.ts`, and `create-api.ts`.

## License

[AGPL-3.0-or-later](../../../../LICENSE)
