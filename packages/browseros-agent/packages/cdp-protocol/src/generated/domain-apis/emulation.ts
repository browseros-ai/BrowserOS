// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type {
  AddScreenParams,
  AddScreenResult,
  CanEmulateResult,
  GetOverriddenSensorInformationParams,
  GetOverriddenSensorInformationResult,
  GetScreenInfosResult,
  RemoveScreenParams,
  ScreenOrientationLockChangedEvent,
  SetAutoDarkModeOverrideParams,
  SetAutomationOverrideParams,
  SetCPUThrottlingRateParams,
  SetDataSaverOverrideParams,
  SetDefaultBackgroundColorOverrideParams,
  SetDeviceMetricsOverrideParams,
  SetDevicePostureOverrideParams,
  SetDisabledImageTypesParams,
  SetDisplayFeaturesOverrideParams,
  SetDocumentCookieDisabledParams,
  SetEmitTouchEventsForMouseParams,
  SetEmulatedMediaParams,
  SetEmulatedOSTextScaleParams,
  SetEmulatedVisionDeficiencyParams,
  SetFocusEmulationEnabledParams,
  SetGeolocationOverrideParams,
  SetHardwareConcurrencyOverrideParams,
  SetIdleOverrideParams,
  SetLocaleOverrideParams,
  SetNavigatorOverridesParams,
  SetPageScaleFactorParams,
  SetPressureDataOverrideParams,
  SetPressureSourceOverrideEnabledParams,
  SetPressureStateOverrideParams,
  SetPrimaryScreenParams,
  SetSafeAreaInsetsOverrideParams,
  SetScriptExecutionDisabledParams,
  SetScrollbarsHiddenParams,
  SetSensorOverrideEnabledParams,
  SetSensorOverrideReadingsParams,
  SetSmallViewportHeightDifferenceOverrideParams,
  SetTimezoneOverrideParams,
  SetTouchEmulationEnabledParams,
  SetUserAgentOverrideParams,
  SetVirtualTimePolicyParams,
  SetVirtualTimePolicyResult,
  SetVisibleSizeParams,
  UpdateScreenParams,
  UpdateScreenResult,
} from '../domains/emulation'

export interface EmulationApi {
  // ── Commands ──

  canEmulate(): Promise<CanEmulateResult>
  clearDeviceMetricsOverride(): Promise<void>
  clearGeolocationOverride(): Promise<void>
  resetPageScaleFactor(): Promise<void>
  setFocusEmulationEnabled(
    params: SetFocusEmulationEnabledParams,
  ): Promise<void>
  setAutoDarkModeOverride(params?: SetAutoDarkModeOverrideParams): Promise<void>
  setCPUThrottlingRate(params: SetCPUThrottlingRateParams): Promise<void>
  setDefaultBackgroundColorOverride(
    params?: SetDefaultBackgroundColorOverrideParams,
  ): Promise<void>
  setSafeAreaInsetsOverride(
    params: SetSafeAreaInsetsOverrideParams,
  ): Promise<void>
  setDeviceMetricsOverride(
    params: SetDeviceMetricsOverrideParams,
  ): Promise<void>
  setDevicePostureOverride(
    params: SetDevicePostureOverrideParams,
  ): Promise<void>
  clearDevicePostureOverride(): Promise<void>
  setDisplayFeaturesOverride(
    params: SetDisplayFeaturesOverrideParams,
  ): Promise<void>
  clearDisplayFeaturesOverride(): Promise<void>
  setScrollbarsHidden(params: SetScrollbarsHiddenParams): Promise<void>
  setDocumentCookieDisabled(
    params: SetDocumentCookieDisabledParams,
  ): Promise<void>
  setEmitTouchEventsForMouse(
    params: SetEmitTouchEventsForMouseParams,
  ): Promise<void>
  setEmulatedMedia(params?: SetEmulatedMediaParams): Promise<void>
  setEmulatedVisionDeficiency(
    params: SetEmulatedVisionDeficiencyParams,
  ): Promise<void>
  setEmulatedOSTextScale(params?: SetEmulatedOSTextScaleParams): Promise<void>
  setGeolocationOverride(params?: SetGeolocationOverrideParams): Promise<void>
  getOverriddenSensorInformation(
    params: GetOverriddenSensorInformationParams,
  ): Promise<GetOverriddenSensorInformationResult>
  setSensorOverrideEnabled(
    params: SetSensorOverrideEnabledParams,
  ): Promise<void>
  setSensorOverrideReadings(
    params: SetSensorOverrideReadingsParams,
  ): Promise<void>
  setPressureSourceOverrideEnabled(
    params: SetPressureSourceOverrideEnabledParams,
  ): Promise<void>
  setPressureStateOverride(
    params: SetPressureStateOverrideParams,
  ): Promise<void>
  setPressureDataOverride(params: SetPressureDataOverrideParams): Promise<void>
  setIdleOverride(params: SetIdleOverrideParams): Promise<void>
  clearIdleOverride(): Promise<void>
  setNavigatorOverrides(params: SetNavigatorOverridesParams): Promise<void>
  setPageScaleFactor(params: SetPageScaleFactorParams): Promise<void>
  setScriptExecutionDisabled(
    params: SetScriptExecutionDisabledParams,
  ): Promise<void>
  setTouchEmulationEnabled(
    params: SetTouchEmulationEnabledParams,
  ): Promise<void>
  setVirtualTimePolicy(
    params: SetVirtualTimePolicyParams,
  ): Promise<SetVirtualTimePolicyResult>
  setLocaleOverride(params?: SetLocaleOverrideParams): Promise<void>
  setTimezoneOverride(params: SetTimezoneOverrideParams): Promise<void>
  setVisibleSize(params: SetVisibleSizeParams): Promise<void>
  setDisabledImageTypes(params: SetDisabledImageTypesParams): Promise<void>
  setDataSaverOverride(params?: SetDataSaverOverrideParams): Promise<void>
  setHardwareConcurrencyOverride(
    params: SetHardwareConcurrencyOverrideParams,
  ): Promise<void>
  setUserAgentOverride(params: SetUserAgentOverrideParams): Promise<void>
  setAutomationOverride(params: SetAutomationOverrideParams): Promise<void>
  setSmallViewportHeightDifferenceOverride(
    params: SetSmallViewportHeightDifferenceOverrideParams,
  ): Promise<void>
  getScreenInfos(): Promise<GetScreenInfosResult>
  addScreen(params: AddScreenParams): Promise<AddScreenResult>
  updateScreen(params: UpdateScreenParams): Promise<UpdateScreenResult>
  removeScreen(params: RemoveScreenParams): Promise<void>
  setPrimaryScreen(params: SetPrimaryScreenParams): Promise<void>

  // ── Events ──

  on(event: 'virtualTimeBudgetExpired', handler: () => void): () => void
  on(
    event: 'screenOrientationLockChanged',
    handler: (params: ScreenOrientationLockChangedEvent) => void,
  ): () => void
}
