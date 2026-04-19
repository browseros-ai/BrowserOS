/**
 * Desktop Control Module — barrel re-export.
 *
 * @module desktop-control
 */

// Types
export type {
  Point,
  ScreenRegion,
  DisplayInfo,
  MouseButton,
  ClickType,
  MouseMoveOptions,
  MouseClickOptions,
  MouseDragOptions,
  ScrollDirection,
  MouseScrollOptions,
  KeyModifier,
  KeyCombination,
  TypeTextOptions,
  ScreenshotFormat,
  ScreenshotOptions,
  ScreenshotResult,
  FileDialogAction,
  FileDialogOptions,
  FileDialogResult,
  DesktopActionType,
  DesktopAction,
  VisionAnalysisRequest,
  VisionAnalysisResponse,
  OrchestratorConfig,
  OrchestratorResult,
  DesktopControlService,
} from './types'

// Services
export { DesktopScreenshotService } from './screenshot'
export { DesktopMouseService } from './mouse'
export { DesktopKeyboardService } from './keyboard'
export { FileDialogService } from './file-dialog'
export { DesktopOrchestrator } from './orchestrator'

// Phase 4B — Smart Coordinator + Integration
export { DialogDetector, DialogState } from './dialog-detector'
export type { DialogDetectionResult, DialogDetectionOptions } from './dialog-detector'
export { FileManager } from './file-manager'
export type { FileInfo, ListFilesOptions, FileSearchResult } from './file-manager'
export {
  UnifiedCoordinator,
  CoordinatorMode,
  TaskCategory,
} from './unified-coordinator'
export type {
  UnifiedTask,
  CoordinatorResult,
  CoordinatorStep,
  UnifiedCoordinatorConfig,
  BrowserControl,
} from './unified-coordinator'

// Phase 4C — Application Launcher + Desktop Explorer + OS Commands
export { AppLauncher } from './app-launcher'
export type {
  LaunchResult,
  RunningAppInfo,
  AppStatusResult,
  QuitResult,
} from './app-launcher'
export { DesktopExplorer } from './desktop-explorer'
export type {
  DetailedFileInfo,
  OpenResult,
  FileOperationResult,
  QuickDir,
} from './desktop-explorer'
export { OSCommands } from './os-commands'
export type {
  OpenUrlResult,
  FileTransferResult,
  ExtractResult,
  SystemInfo,
  DiskInfo,
  ClipboardResult,
  ControlResult,
} from './os-commands'

// Phase 4D — Agent Tools Registration + Plugin
export {
  desktopTools,
  desktop_screenshot,
  desktop_click,
  desktop_type,
  desktop_hotkey,
  desktop_scroll,
  desktop_drag,
  desktop_open_file,
  desktop_open_folder,
  desktop_open_app,
  desktop_close_app,
  desktop_list_apps,
  desktop_list_files,
  desktop_search_files,
  desktop_copy_file,
  desktop_move_file,
  desktop_delete_file,
  desktop_clipboard_read,
  desktop_clipboard_write,
  desktop_system_info,
  desktop_file_upload,
  desktop_file_download,
  getDesktopServices,
  resetDesktopServices,
} from './desktop-agent-tools'
export type { DesktopToolServices } from './desktop-agent-tools'

export {
  DesktopPlugin,
  getDesktopPlugin,
  resetDesktopPlugin,
} from './desktop-plugin'
export type {
  DesktopCapabilityStatus,
  DesktopHealthReport,
  PluginEvent,
  PluginEventListener,
} from './desktop-plugin'
