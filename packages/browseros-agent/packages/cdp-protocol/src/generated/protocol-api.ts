// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { AccessibilityApi } from './domain-apis/accessibility'
import type { AnimationApi } from './domain-apis/animation'
import type { AuditsApi } from './domain-apis/audits'
import type { AutofillApi } from './domain-apis/autofill'
import type { BackgroundServiceApi } from './domain-apis/background-service'
import type { BluetoothEmulationApi } from './domain-apis/bluetooth-emulation'
import type { BookmarksApi } from './domain-apis/bookmarks'
import type { BrowserApi } from './domain-apis/browser'
import type { CacheStorageApi } from './domain-apis/cache-storage'
import type { CastApi } from './domain-apis/cast'
import type { ConsoleApi } from './domain-apis/console'
import type { CrashReportContextApi } from './domain-apis/crash-report-context'
import type { CSSApi } from './domain-apis/css'
import type { DebuggerApi } from './domain-apis/debugger'
import type { DeviceAccessApi } from './domain-apis/device-access'
import type { DeviceOrientationApi } from './domain-apis/device-orientation'
import type { DOMApi } from './domain-apis/dom'
import type { DOMDebuggerApi } from './domain-apis/dom-debugger'
import type { DOMSnapshotApi } from './domain-apis/dom-snapshot'
import type { DOMStorageApi } from './domain-apis/dom-storage'
import type { EmulationApi } from './domain-apis/emulation'
import type { EventBreakpointsApi } from './domain-apis/event-breakpoints'
import type { ExtensionsApi } from './domain-apis/extensions'
import type { FedCmApi } from './domain-apis/fed-cm'
import type { FetchApi } from './domain-apis/fetch'
import type { FileSystemApi } from './domain-apis/file-system'
import type { HeadlessExperimentalApi } from './domain-apis/headless-experimental'
import type { HeapProfilerApi } from './domain-apis/heap-profiler'
import type { HistoryApi } from './domain-apis/history'
import type { IndexedDBApi } from './domain-apis/indexed-db'
import type { InputApi } from './domain-apis/input'
import type { InspectorApi } from './domain-apis/inspector'
import type { IOApi } from './domain-apis/io'
import type { LayerTreeApi } from './domain-apis/layer-tree'
import type { LogApi } from './domain-apis/log'
import type { MediaApi } from './domain-apis/media'
import type { MemoryApi } from './domain-apis/memory'
import type { NetworkApi } from './domain-apis/network'
import type { OverlayApi } from './domain-apis/overlay'
import type { PageApi } from './domain-apis/page'
import type { PerformanceApi } from './domain-apis/performance'
import type { PerformanceTimelineApi } from './domain-apis/performance-timeline'
import type { PreloadApi } from './domain-apis/preload'
import type { ProfilerApi } from './domain-apis/profiler'
import type { PWAApi } from './domain-apis/pwa'
import type { RuntimeApi } from './domain-apis/runtime'
import type { SchemaApi } from './domain-apis/schema'
import type { SecurityApi } from './domain-apis/security'
import type { ServiceWorkerApi } from './domain-apis/service-worker'
import type { SmartCardEmulationApi } from './domain-apis/smart-card-emulation'
import type { StorageApi } from './domain-apis/storage'
import type { SystemInfoApi } from './domain-apis/system-info'
import type { TargetApi } from './domain-apis/target'
import type { TetheringApi } from './domain-apis/tethering'
import type { TracingApi } from './domain-apis/tracing'
import type { WebAudioApi } from './domain-apis/web-audio'
import type { WebAuthnApi } from './domain-apis/web-authn'
import type { WebMCPApi } from './domain-apis/web-mcp'

export interface ProtocolApi {
  readonly Accessibility: AccessibilityApi
  readonly Animation: AnimationApi
  readonly Audits: AuditsApi
  readonly Autofill: AutofillApi
  readonly Bookmarks: BookmarksApi
  readonly BackgroundService: BackgroundServiceApi
  readonly BluetoothEmulation: BluetoothEmulationApi
  readonly Browser: BrowserApi
  readonly CSS: CSSApi
  readonly CacheStorage: CacheStorageApi
  readonly Cast: CastApi
  readonly CrashReportContext: CrashReportContextApi
  readonly DOM: DOMApi
  readonly DOMDebugger: DOMDebuggerApi
  readonly DOMSnapshot: DOMSnapshotApi
  readonly DOMStorage: DOMStorageApi
  readonly DeviceAccess: DeviceAccessApi
  readonly DeviceOrientation: DeviceOrientationApi
  readonly Emulation: EmulationApi
  readonly EventBreakpoints: EventBreakpointsApi
  readonly Extensions: ExtensionsApi
  readonly FedCm: FedCmApi
  readonly Fetch: FetchApi
  readonly FileSystem: FileSystemApi
  readonly HeadlessExperimental: HeadlessExperimentalApi
  readonly History: HistoryApi
  readonly IO: IOApi
  readonly IndexedDB: IndexedDBApi
  readonly Input: InputApi
  readonly Inspector: InspectorApi
  readonly LayerTree: LayerTreeApi
  readonly Log: LogApi
  readonly Media: MediaApi
  readonly Memory: MemoryApi
  readonly Network: NetworkApi
  readonly Overlay: OverlayApi
  readonly PWA: PWAApi
  readonly Page: PageApi
  readonly Performance: PerformanceApi
  readonly PerformanceTimeline: PerformanceTimelineApi
  readonly Preload: PreloadApi
  readonly Security: SecurityApi
  readonly ServiceWorker: ServiceWorkerApi
  readonly SmartCardEmulation: SmartCardEmulationApi
  readonly Storage: StorageApi
  readonly SystemInfo: SystemInfoApi
  readonly Target: TargetApi
  readonly Tethering: TetheringApi
  readonly Tracing: TracingApi
  readonly WebAudio: WebAudioApi
  readonly WebAuthn: WebAuthnApi
  readonly WebMCP: WebMCPApi
  readonly Console: ConsoleApi
  readonly Debugger: DebuggerApi
  readonly HeapProfiler: HeapProfilerApi
  readonly Profiler: ProfilerApi
  readonly Runtime: RuntimeApi
  readonly Schema: SchemaApi
}
