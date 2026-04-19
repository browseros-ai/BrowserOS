/**
 * Desktop Control Module — barrel re-export.
 *
 * @module desktop-control
 */

export type {
  AppStatusResult,
  LaunchResult,
  QuitResult,
  RunningAppInfo,
} from './app-launcher'
// Phase 4C — Application Launcher + Desktop Explorer + OS Commands
export { AppLauncher } from './app-launcher'
export type { DesktopToolServices } from './desktop-agent-tools'
// Phase 4D — Agent Tools Registration + Plugin
export {
  desktop_click,
  desktop_clipboard_read,
  desktop_clipboard_write,
  desktop_close_app,
  desktop_copy_file,
  desktop_delete_file,
  desktop_drag,
  desktop_file_download,
  desktop_file_upload,
  desktop_hotkey,
  desktop_list_apps,
  desktop_list_files,
  desktop_move_file,
  desktop_open_app,
  desktop_open_file,
  desktop_open_folder,
  desktop_screenshot,
  desktop_scroll,
  desktop_search_files,
  desktop_system_info,
  desktop_type,
  desktopTools,
  getDesktopServices,
  resetDesktopServices,
} from './desktop-agent-tools'
export type {
  DetailedFileInfo,
  FileOperationResult,
  OpenResult,
  QuickDir,
} from './desktop-explorer'
export { DesktopExplorer } from './desktop-explorer'
export type {
  DesktopCapabilityStatus,
  DesktopHealthReport,
  PluginEvent,
  PluginEventListener,
} from './desktop-plugin'
export {
  DesktopPlugin,
  getDesktopPlugin,
  resetDesktopPlugin,
} from './desktop-plugin'
export type {
  DialogDetectionOptions,
  DialogDetectionResult,
} from './dialog-detector'
// Phase 4B — Smart Coordinator + Integration
export { DialogDetector, DialogState } from './dialog-detector'
export { FileDialogService } from './file-dialog'
export type {
  FileInfo,
  FileSearchResult,
  ListFilesOptions,
} from './file-manager'
export { FileManager } from './file-manager'
export { DesktopKeyboardService } from './keyboard'
export { DesktopMouseService } from './mouse'
export { DesktopOrchestrator } from './orchestrator'
export type {
  ClipboardResult,
  ControlResult,
  DiskInfo,
  ExtractResult,
  FileTransferResult,
  OpenUrlResult,
  SystemInfo,
} from './os-commands'
export { OSCommands } from './os-commands'
// Services
export { DesktopScreenshotService } from './screenshot'
// Types
export type {
  ClickType,
  DesktopAction,
  DesktopActionType,
  DesktopControlService,
  DisplayInfo,
  FileDialogAction,
  FileDialogOptions,
  FileDialogResult,
  KeyCombination,
  KeyModifier,
  MouseButton,
  MouseClickOptions,
  MouseDragOptions,
  MouseMoveOptions,
  MouseScrollOptions,
  OrchestratorConfig,
  OrchestratorResult,
  Point,
  ScreenRegion,
  ScreenshotFormat,
  ScreenshotOptions,
  ScreenshotResult,
  ScrollDirection,
  TypeTextOptions,
  VisionAnalysisRequest,
  VisionAnalysisResponse,
} from './types'
export type {
  BrowserControl,
  CoordinatorResult,
  CoordinatorStep,
  UnifiedCoordinatorConfig,
  UnifiedTask,
} from './unified-coordinator'
export {
  CoordinatorMode,
  TaskCategory,
  UnifiedCoordinator,
} from './unified-coordinator'
