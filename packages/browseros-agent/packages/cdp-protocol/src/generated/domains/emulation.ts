// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { RGBA } from './dom'
import type { TimeSinceEpoch } from './network'
import type { Viewport } from './page'

// ══ Types ══

export interface SafeAreaInsets {
  top?: number
  topMax?: number
  left?: number
  leftMax?: number
  bottom?: number
  bottomMax?: number
  right?: number
  rightMax?: number
}

export interface ScreenOrientation {
  type:
    | 'portraitPrimary'
    | 'portraitSecondary'
    | 'landscapePrimary'
    | 'landscapeSecondary'
  angle: number
}

export interface DisplayFeature {
  orientation: 'vertical' | 'horizontal'
  offset: number
  maskLength: number
}

export interface DevicePosture {
  type: 'continuous' | 'folded'
}

export interface MediaFeature {
  name: string
  value: string
}

export type VirtualTimePolicy =
  | 'advance'
  | 'pause'
  | 'pauseIfNetworkFetchesPending'

export interface UserAgentBrandVersion {
  brand: string
  version: string
}

export interface UserAgentMetadata {
  brands?: UserAgentBrandVersion[]
  fullVersionList?: UserAgentBrandVersion[]
  fullVersion?: string
  platform: string
  platformVersion: string
  architecture: string
  model: string
  mobile: boolean
  bitness?: string
  wow64?: boolean
  formFactors?: string[]
}

export type SensorType =
  | 'absolute-orientation'
  | 'accelerometer'
  | 'ambient-light'
  | 'gravity'
  | 'gyroscope'
  | 'linear-acceleration'
  | 'magnetometer'
  | 'relative-orientation'

export interface SensorMetadata {
  available?: boolean
  minimumFrequency?: number
  maximumFrequency?: number
}

export interface SensorReadingSingle {
  value: number
}

export interface SensorReadingXYZ {
  x: number
  y: number
  z: number
}

export interface SensorReadingQuaternion {
  x: number
  y: number
  z: number
  w: number
}

export interface SensorReading {
  single?: SensorReadingSingle
  xyz?: SensorReadingXYZ
  quaternion?: SensorReadingQuaternion
}

export type PressureSource = 'cpu'

export type PressureState = 'nominal' | 'fair' | 'serious' | 'critical'

export interface PressureMetadata {
  available?: boolean
}

export interface WorkAreaInsets {
  top?: number
  left?: number
  bottom?: number
  right?: number
}

export type ScreenId = string

export interface ScreenInfo {
  left: number
  top: number
  width: number
  height: number
  availLeft: number
  availTop: number
  availWidth: number
  availHeight: number
  devicePixelRatio: number
  orientation: ScreenOrientation
  colorDepth: number
  isExtended: boolean
  isInternal: boolean
  isPrimary: boolean
  label: string
  id: ScreenId
}

export type DisabledImageType = 'avif' | 'jxl' | 'webp'

// ══ Commands ══

export interface CanEmulateResult {
  result: boolean
}

export interface SetFocusEmulationEnabledParams {
  enabled: boolean
}

export interface SetAutoDarkModeOverrideParams {
  enabled?: boolean
}

export interface SetCPUThrottlingRateParams {
  rate: number
}

export interface SetDefaultBackgroundColorOverrideParams {
  color?: RGBA
}

export interface SetSafeAreaInsetsOverrideParams {
  insets: SafeAreaInsets
}

export interface SetDeviceMetricsOverrideParams {
  width: number
  height: number
  deviceScaleFactor: number
  mobile: boolean
  scale?: number
  screenWidth?: number
  screenHeight?: number
  positionX?: number
  positionY?: number
  dontSetVisibleSize?: boolean
  screenOrientation?: ScreenOrientation
  viewport?: Viewport
  displayFeature?: DisplayFeature
  devicePosture?: DevicePosture
  scrollbarType?: 'overlay' | 'default'
  screenOrientationLockEmulation?: boolean
}

export interface SetDevicePostureOverrideParams {
  posture: DevicePosture
}

export interface SetDisplayFeaturesOverrideParams {
  features: DisplayFeature[]
}

export interface SetScrollbarsHiddenParams {
  hidden: boolean
}

export interface SetDocumentCookieDisabledParams {
  disabled: boolean
}

export interface SetEmitTouchEventsForMouseParams {
  enabled: boolean
  configuration?: 'mobile' | 'desktop'
}

export interface SetEmulatedMediaParams {
  media?: string
  features?: MediaFeature[]
}

export interface SetEmulatedVisionDeficiencyParams {
  type:
    | 'none'
    | 'blurredVision'
    | 'reducedContrast'
    | 'achromatopsia'
    | 'deuteranopia'
    | 'protanopia'
    | 'tritanopia'
}

export interface SetEmulatedOSTextScaleParams {
  scale?: number
}

export interface SetGeolocationOverrideParams {
  latitude?: number
  longitude?: number
  accuracy?: number
  altitude?: number
  altitudeAccuracy?: number
  heading?: number
  speed?: number
}

export interface GetOverriddenSensorInformationParams {
  type: SensorType
}

export interface GetOverriddenSensorInformationResult {
  requestedSamplingFrequency: number
}

export interface SetSensorOverrideEnabledParams {
  enabled: boolean
  type: SensorType
  metadata?: SensorMetadata
}

export interface SetSensorOverrideReadingsParams {
  type: SensorType
  reading: SensorReading
}

export interface SetPressureSourceOverrideEnabledParams {
  enabled: boolean
  source: PressureSource
  metadata?: PressureMetadata
}

export interface SetPressureStateOverrideParams {
  source: PressureSource
  state: PressureState
}

export interface SetPressureDataOverrideParams {
  source: PressureSource
  state: PressureState
  ownContributionEstimate?: number
}

export interface SetIdleOverrideParams {
  isUserActive: boolean
  isScreenUnlocked: boolean
}

export interface SetNavigatorOverridesParams {
  platform: string
}

export interface SetPageScaleFactorParams {
  pageScaleFactor: number
}

export interface SetScriptExecutionDisabledParams {
  value: boolean
}

export interface SetTouchEmulationEnabledParams {
  enabled: boolean
  maxTouchPoints?: number
}

export interface SetVirtualTimePolicyParams {
  policy: VirtualTimePolicy
  budget?: number
  maxVirtualTimeTaskStarvationCount?: number
  initialVirtualTime?: TimeSinceEpoch
}

export interface SetVirtualTimePolicyResult {
  virtualTimeTicksBase: number
}

export interface SetLocaleOverrideParams {
  locale?: string
}

export interface SetTimezoneOverrideParams {
  timezoneId: string
}

export interface SetVisibleSizeParams {
  width: number
  height: number
}

export interface SetDisabledImageTypesParams {
  imageTypes: DisabledImageType[]
}

export interface SetDataSaverOverrideParams {
  dataSaverEnabled?: boolean
}

export interface SetHardwareConcurrencyOverrideParams {
  hardwareConcurrency: number
}

export interface SetUserAgentOverrideParams {
  userAgent: string
  acceptLanguage?: string
  platform?: string
  userAgentMetadata?: UserAgentMetadata
}

export interface SetAutomationOverrideParams {
  enabled: boolean
}

export interface SetSmallViewportHeightDifferenceOverrideParams {
  difference: number
}

export interface GetScreenInfosResult {
  screenInfos: ScreenInfo[]
}

export interface AddScreenParams {
  left: number
  top: number
  width: number
  height: number
  workAreaInsets?: WorkAreaInsets
  devicePixelRatio?: number
  rotation?: number
  colorDepth?: number
  label?: string
  isInternal?: boolean
}

export interface AddScreenResult {
  screenInfo: ScreenInfo
}

export interface UpdateScreenParams {
  screenId: ScreenId
  left?: number
  top?: number
  width?: number
  height?: number
  workAreaInsets?: WorkAreaInsets
  devicePixelRatio?: number
  rotation?: number
  colorDepth?: number
  label?: string
  isInternal?: boolean
}

export interface UpdateScreenResult {
  screenInfo: ScreenInfo
}

export interface RemoveScreenParams {
  screenId: ScreenId
}

export interface SetPrimaryScreenParams {
  screenId: ScreenId
}

// ══ Events ══

export interface ScreenOrientationLockChangedEvent {
  locked: boolean
  orientation?: ScreenOrientation
}
