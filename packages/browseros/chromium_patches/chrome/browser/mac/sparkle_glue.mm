diff --git a/chrome/browser/mac/sparkle_glue.mm b/chrome/browser/mac/sparkle_glue.mm
new file mode 100644
index 0000000000000..25a2dd2d5a578
--- /dev/null
+++ b/chrome/browser/mac/sparkle_glue.mm
@@ -0,0 +1,662 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#import "chrome/browser/mac/sparkle_glue.h"
+
+#include <sys/mount.h>
+#include <sys/stat.h>
+
+#include "base/apple/bundle_locations.h"
+#include "base/command_line.h"
+#include "base/logging.h"
+#include "base/strings/sys_string_conversions.h"
+#include "base/system/sys_info.h"
+#include "base/version.h"
+#include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_switches.h"
+#include "chrome/browser/upgrade_detector/build_state.h"
+
+#import <Sparkle/Sparkle.h>
+
+#if !defined(__has_feature) || !__has_feature(objc_arc)
+#error "This file requires ARC support."
+#endif
+
+namespace {
+
+NSString* GetArchitectureSpecificFeedURL() {
+  const char* kBaseURL = "https://cdn.browseros.com/";
+
+  if (base::SysInfo::OperatingSystemArchitecture() == "x86_64") {
+    return [NSString stringWithFormat:@"%sappcast-x86_64.xml", kBaseURL];
+  }
+  return [NSString stringWithFormat:@"%sappcast.xml", kBaseURL];
+}
+
+bool IsOnReadOnlyFilesystem(NSString* path) {
+  struct statfs statfs_buf;
+  if (statfs(path.fileSystemRepresentation, &statfs_buf) != 0) {
+    return false;
+  }
+  return (statfs_buf.f_flags & MNT_RDONLY) != 0;
+}
+
+// Notify the Chromium upgrade system that a Sparkle update is ready.
+// This triggers the app menu badge to appear.
+void NotifyUpgradeReady(const std::string& version) {
+  if (!g_browser_process) {
+    LOG(WARNING) << "Sparkle: Cannot notify upgrade - no browser process";
+    return;
+  }
+
+  BuildState* build_state = g_browser_process->GetBuildState();
+  if (!build_state) {
+    LOG(WARNING) << "Sparkle: Cannot notify upgrade - no build state";
+    return;
+  }
+
+  VLOG(1) << "Sparkle: Notifying upgrade system, version " << version;
+  build_state->SetUpdate(BuildState::UpdateType::kNormalUpdate,
+                         base::Version(version), std::nullopt);
+}
+
+}  // namespace
+
+#pragma mark - SparkleProgress
+
+@implementation SparkleProgress {
+  uint64_t _bytesReceived;
+  uint64_t _bytesTotal;
+}
+
+- (instancetype)initWithReceived:(uint64_t)received total:(uint64_t)total {
+  if (self = [super init]) {
+    _bytesReceived = received;
+    _bytesTotal = total;
+  }
+  return self;
+}
+
+- (uint64_t)bytesReceived {
+  return _bytesReceived;
+}
+
+- (uint64_t)bytesTotal {
+  return _bytesTotal;
+}
+
+- (double)fraction {
+  if (_bytesTotal == 0) {
+    return 0.0;
+  }
+  return static_cast<double>(_bytesReceived) / static_cast<double>(_bytesTotal);
+}
+
+- (int)percentage {
+  return static_cast<int>(self.fraction * 100.0);
+}
+
+@end
+
+#pragma mark - Forward Declarations
+
+// Forward declare internal SparkleGlue methods used by BrowserOSUserDriver.
+@interface SparkleGlue ()
+- (void)setInternalStatus:(SparkleStatus)status;
+- (void)setInternalStatus:(SparkleStatus)status
+             errorMessage:(nullable NSString*)errorMessage;
+- (void)notifyProgress:(SparkleProgress*)progress;
+@end
+
+#pragma mark - BrowserOSUserDriver
+
+// Custom SPUUserDriver that captures all progress callbacks and forwards
+// them to SparkleGlue for distribution to observers.
+@interface BrowserOSUserDriver : NSObject <SPUUserDriver>
+
+@property(nonatomic, weak) SparkleGlue* glue;
+@property(nonatomic) uint64_t expectedBytes;
+@property(nonatomic) uint64_t receivedBytes;
+@property(nonatomic, copy, nullable) void (^installReplyBlock)(SPUUserUpdateChoice);
+@property(nonatomic, copy, nullable) void (^downloadCancellation)(void);
+@property(nonatomic, copy, nullable) NSString* updateVersion;
+
+@end
+
+@implementation BrowserOSUserDriver
+
+@synthesize glue = _glue;
+@synthesize expectedBytes = _expectedBytes;
+@synthesize receivedBytes = _receivedBytes;
+@synthesize installReplyBlock = _installReplyBlock;
+@synthesize downloadCancellation = _downloadCancellation;
+@synthesize updateVersion = _updateVersion;
+
+#pragma mark - SPUUserDriver Required Methods
+
+- (void)showUpdatePermissionRequest:(SPUUpdatePermissionRequest*)request
+                              reply:(void (^)(SUUpdatePermissionResponse*))reply {
+  // Auto-grant permission. The Info.plist has SUEnableAutomaticChecks=YES.
+  SUUpdatePermissionResponse* response =
+      [[SUUpdatePermissionResponse alloc] initWithAutomaticUpdateChecks:YES
+                                                sendSystemProfile:NO];
+  reply(response);
+}
+
+- (void)showUserInitiatedUpdateCheckWithCancellation:(void (^)(void))cancellation {
+  VLOG(1) << "Sparkle: User initiated update check";
+  [self.glue setInternalStatus:SparkleStatusChecking];
+}
+
+- (void)showUpdateFoundWithAppcastItem:(SUAppcastItem*)appcastItem
+                                 state:(SPUUserUpdateState*)state
+                                 reply:(void (^)(SPUUserUpdateChoice))reply {
+  VLOG(1) << "Sparkle: Update found - "
+          << base::SysNSStringToUTF8(appcastItem.displayVersionString);
+
+  // Store version for upgrade notification (use display version, not internal).
+  self.updateVersion = appcastItem.displayVersionString;
+
+  if (appcastItem.informationOnlyUpdate) {
+    // Information-only updates cannot be installed directly.
+    reply(SPUUserUpdateChoiceDismiss);
+    return;
+  }
+
+  switch (state.stage) {
+    case SPUUserUpdateStageNotDownloaded:
+      // Start downloading the update.
+      reply(SPUUserUpdateChoiceInstall);
+      break;
+
+    case SPUUserUpdateStageDownloaded:
+      // Update already downloaded, ready to install.
+      self.installReplyBlock = reply;
+      [self.glue setInternalStatus:SparkleStatusReadyToInstall];
+      NotifyUpgradeReady(base::SysNSStringToUTF8(self.updateVersion));
+      break;
+
+    case SPUUserUpdateStageInstalling:
+      // Already installing - store reply block and notify user, don't auto-proceed
+      self.installReplyBlock = reply;
+      [self.glue setInternalStatus:SparkleStatusReadyToInstall];
+      NotifyUpgradeReady(base::SysNSStringToUTF8(self.updateVersion));
+      break;
+  }
+}
+
+- (void)showUpdateReleaseNotesWithDownloadData:(SPUDownloadData*)downloadData {
+  // Release notes display handled by Chrome's UI if needed.
+}
+
+- (void)showUpdateReleaseNotesFailedToDownloadWithError:(NSError*)error {
+  LOG(WARNING) << "Sparkle: Failed to download release notes: "
+               << base::SysNSStringToUTF8(error.localizedDescription);
+}
+
+- (void)showUpdateNotFoundWithError:(NSError*)error
+                    acknowledgement:(void (^)(void))acknowledgement {
+  VLOG(1) << "Sparkle: No update found";
+  [self.glue setInternalStatus:SparkleStatusUpToDate];
+  acknowledgement();
+}
+
+- (void)showUpdaterError:(NSError*)error
+         acknowledgement:(void (^)(void))acknowledgement {
+  LOG(ERROR) << "Sparkle: Update error: "
+             << base::SysNSStringToUTF8(error.localizedDescription);
+  [self.glue setInternalStatus:SparkleStatusError
+                  errorMessage:error.localizedDescription];
+  acknowledgement();
+}
+
+- (void)showDownloadInitiatedWithCancellation:(void (^)(void))cancellation {
+  VLOG(1) << "Sparkle: Download initiated";
+  self.downloadCancellation = cancellation;
+  self.expectedBytes = 0;
+  self.receivedBytes = 0;
+  [self.glue setInternalStatus:SparkleStatusDownloading];
+}
+
+- (void)showDownloadDidReceiveExpectedContentLength:(uint64_t)expectedContentLength {
+  VLOG(1) << "Sparkle: Expected download size: " << expectedContentLength;
+  self.expectedBytes = expectedContentLength;
+  self.receivedBytes = 0;
+}
+
+- (void)showDownloadDidReceiveDataOfLength:(uint64_t)length {
+  self.receivedBytes += length;
+
+  SparkleProgress* progress =
+      [[SparkleProgress alloc] initWithReceived:self.receivedBytes
+                                          total:self.expectedBytes];
+  VLOG(2) << "Sparkle: Download progress: " << progress.percentage << "%";
+  [self.glue notifyProgress:progress];
+}
+
+- (void)showDownloadDidStartExtractingUpdate {
+  VLOG(1) << "Sparkle: Extraction started";
+  [self.glue setInternalStatus:SparkleStatusExtracting];
+}
+
+- (void)showExtractionReceivedProgress:(double)progress {
+  SparkleProgress* progressObj =
+      [[SparkleProgress alloc] initWithReceived:static_cast<uint64_t>(progress * 100)
+                                          total:100];
+  VLOG(2) << "Sparkle: Extraction progress: " << progressObj.percentage << "%";
+  [self.glue notifyProgress:progressObj];
+}
+
+- (void)showReadyToInstallAndRelaunch:(void (^)(SPUUserUpdateChoice))reply {
+  VLOG(1) << "Sparkle: Ready to install and relaunch";
+  self.installReplyBlock = reply;
+  [self.glue setInternalStatus:SparkleStatusReadyToInstall];
+  if (self.updateVersion) {
+    NotifyUpgradeReady(base::SysNSStringToUTF8(self.updateVersion));
+  }
+}
+
+- (void)showInstallingUpdateWithApplicationTerminated:(BOOL)applicationTerminated
+                          retryTerminatingApplication:(void (^)(void))retryTerminatingApplication {
+  VLOG(1) << "Sparkle: Installing update (app terminated: "
+          << (applicationTerminated ? "yes" : "no") << ")";
+  [self.glue setInternalStatus:SparkleStatusInstalling];
+}
+
+- (void)showUpdateInstalledAndRelaunched:(BOOL)relaunched
+                         acknowledgement:(void (^)(void))acknowledgement {
+  VLOG(1) << "Sparkle: Update installed (relaunched: "
+          << (relaunched ? "yes" : "no") << ")";
+  acknowledgement();
+}
+
+- (void)showUpdateInFocus {
+  // No UI to bring to focus - Chrome handles this.
+}
+
+- (void)dismissUpdateInstallation {
+  VLOG(1) << "Sparkle: Update installation dismissed";
+  self.installReplyBlock = nil;
+  self.downloadCancellation = nil;
+  [self.glue setInternalStatus:SparkleStatusIdle];
+}
+
+#pragma mark - Internal
+
+- (void)triggerInstall {
+  if (self.installReplyBlock) {
+    VLOG(1) << "Sparkle: Triggering install";
+    void (^reply)(SPUUserUpdateChoice) = self.installReplyBlock;
+    self.installReplyBlock = nil;
+    reply(SPUUserUpdateChoiceInstall);
+  } else {
+    LOG(WARNING) << "Sparkle: Install requested but no reply block available";
+  }
+}
+
+@end
+
+#pragma mark - SparkleGlue
+
+@interface SparkleGlue () <SPUUpdaterDelegate>
+
+@property(nonatomic, strong) SPUUpdater* updater;
+@property(nonatomic, strong) BrowserOSUserDriver* userDriver;
+@property(nonatomic, strong) NSHashTable<id<SparkleObserver>>* observers;
+@property(nonatomic, readwrite) SparkleStatus status;
+@property(nonatomic, readwrite, copy, nullable) NSString* lastErrorMessage;
+
+#if !defined(OFFICIAL_BUILD)
+@property(nonatomic) BOOL dryRunMode;
+@property(nonatomic, copy, nullable) NSString* spoofedVersion;
+#endif
+
+@end
+
+@implementation SparkleGlue
+
+@synthesize updater = _updater;
+@synthesize userDriver = _userDriver;
+@synthesize observers = _observers;
+@synthesize status = _status;
+@synthesize lastErrorMessage = _lastErrorMessage;
+
+#if !defined(OFFICIAL_BUILD)
+@synthesize dryRunMode = _dryRunMode;
+@synthesize spoofedVersion = _spoofedVersion;
+#endif
+
++ (nullable instancetype)sharedSparkleGlue {
+  static SparkleGlue* instance = nil;
+  static dispatch_once_t onceToken;
+
+  dispatch_once(&onceToken, ^{
+    auto* cmd = base::CommandLine::ForCurrentProcess();
+    if (cmd && cmd->HasSwitch("disable-updates")) {
+      VLOG(1) << "Sparkle: Updates disabled via command line";
+      return;
+    }
+
+    NSString* appPath = base::apple::OuterBundle().bundlePath;
+    if (IsOnReadOnlyFilesystem(appPath)) {
+      VLOG(1) << "Sparkle: Running from read-only filesystem, updates disabled";
+      return;
+    }
+
+    instance = [[SparkleGlue alloc] init];
+  });
+
+  return instance;
+}
+
+- (nullable instancetype)init {
+  if (self = [super init]) {
+    _observers = [NSHashTable weakObjectsHashTable];
+    _status = SparkleStatusIdle;
+
+    if (![self initializeSparkle]) {
+      return nil;
+    }
+
+    [self applyCommandLineFlags];
+    [self maybeForceUpdateCheck];
+  }
+  return self;
+}
+
+- (BOOL)initializeSparkle {
+  _userDriver = [[BrowserOSUserDriver alloc] init];
+  _userDriver.glue = self;
+
+  NSBundle* hostBundle = base::apple::OuterBundle();
+  _updater = [[SPUUpdater alloc] initWithHostBundle:hostBundle
+                                  applicationBundle:hostBundle
+                                         userDriver:_userDriver
+                                           delegate:self];
+
+  NSError* error = nil;
+  if (![_updater startUpdater:&error]) {
+    LOG(ERROR) << "Sparkle: Failed to start updater: "
+               << base::SysNSStringToUTF8(error.localizedDescription);
+    return NO;
+  }
+
+  // Log auto-update configuration for validation.
+  VLOG(1) << "Sparkle: Updater initialized successfully";
+  VLOG(1) << "Sparkle: automaticallyChecksForUpdates="
+          << (_updater.automaticallyChecksForUpdates ? "YES" : "NO");
+  VLOG(1) << "Sparkle: automaticallyDownloadsUpdates="
+          << (_updater.automaticallyDownloadsUpdates ? "YES" : "NO");
+  VLOG(1) << "Sparkle: updateCheckInterval=" << _updater.updateCheckInterval
+          << " seconds";
+  return YES;
+}
+
+- (void)applyCommandLineFlags {
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+  if (!cmd) {
+    return;
+  }
+
+#if !defined(OFFICIAL_BUILD)
+  if (cmd->HasSwitch(browseros::kSparkleDryRun)) {
+    LOG(WARNING) << "Sparkle: DRY-RUN MODE enabled";
+    _dryRunMode = YES;
+  }
+
+  if (cmd->HasSwitch(browseros::kSparkleSpoofVersion)) {
+    std::string version = cmd->GetSwitchValueASCII(browseros::kSparkleSpoofVersion);
+    LOG(WARNING) << "Sparkle: Spoofing version as " << version;
+    _spoofedVersion = base::SysUTF8ToNSString(version);
+  }
+
+  if (cmd->HasSwitch(browseros::kSparkleVerbose)) {
+    [[NSUserDefaults standardUserDefaults] setBool:YES
+                                            forKey:@"SUEnableDebugMode"];
+    VLOG(1) << "Sparkle: Verbose logging enabled";
+  }
+#endif
+}
+
+- (void)maybeForceUpdateCheck {
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+  if (cmd && cmd->HasSwitch(browseros::kSparkleForceCheck)) {
+    VLOG(1) << "Sparkle: Force check triggered via command line";
+    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC),
+                   dispatch_get_main_queue(), ^{
+#if !defined(OFFICIAL_BUILD)
+                     if (self.dryRunMode) {
+                       [self startDryRunSimulation];
+                       return;
+                     }
+#endif
+                     [self checkForUpdates];
+                   });
+  }
+}
+
+#pragma mark - Public API
+
+- (BOOL)updateReady {
+  return _status == SparkleStatusReadyToInstall;
+}
+
+- (void)checkForUpdates {
+#if !defined(OFFICIAL_BUILD)
+  if (_dryRunMode) {
+    [self startDryRunSimulation];
+    return;
+  }
+#endif
+
+  if (!_updater.canCheckForUpdates) {
+    VLOG(1) << "Sparkle: Cannot check for updates right now";
+    return;
+  }
+
+  VLOG(1) << "Sparkle: Checking for updates";
+  [_updater checkForUpdates];
+}
+
+- (void)installAndRelaunch {
+  if (_status != SparkleStatusReadyToInstall) {
+    LOG(WARNING) << "Sparkle: installAndRelaunch called but not ready";
+    return;
+  }
+
+  VLOG(1) << "Sparkle: Installing and relaunching";
+  [_userDriver triggerInstall];
+}
+
+- (void)addObserver:(id<SparkleObserver>)observer {
+  [_observers addObject:observer];
+
+  // Immediately notify of current status.
+  [observer sparkleDidChangeStatus:_status];
+}
+
+- (void)removeObserver:(id<SparkleObserver>)observer {
+  [_observers removeObject:observer];
+}
+
+#pragma mark - Internal Status Management
+
+- (void)setInternalStatus:(SparkleStatus)status {
+  [self setInternalStatus:status errorMessage:nil];
+}
+
+- (void)setInternalStatus:(SparkleStatus)status
+             errorMessage:(nullable NSString*)errorMessage {
+  if (_status == status && !errorMessage) {
+    return;
+  }
+
+  _status = status;
+  _lastErrorMessage = [errorMessage copy];
+
+  VLOG(1) << "Sparkle: Status changed to " << static_cast<int>(status);
+
+  [self notifyStatusChange];
+
+  if (errorMessage && [errorMessage length] > 0) {
+    [self notifyError:errorMessage];
+  }
+}
+
+- (void)notifyStatusChange {
+  for (id<SparkleObserver> observer in _observers) {
+    [observer sparkleDidChangeStatus:_status];
+  }
+}
+
+- (void)notifyProgress:(SparkleProgress*)progress {
+  for (id<SparkleObserver> observer in _observers) {
+    [observer sparkleDidUpdateProgress:progress];
+  }
+}
+
+- (void)notifyError:(NSString*)errorMessage {
+  for (id<SparkleObserver> observer in _observers) {
+    if ([observer respondsToSelector:@selector(sparkleDidFailWithError:)]) {
+      [observer sparkleDidFailWithError:errorMessage];
+    }
+  }
+}
+
+#pragma mark - SPUUpdaterDelegate
+
+- (nullable NSString*)feedURLStringForUpdater:(SPUUpdater*)updater {
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+  if (cmd && cmd->HasSwitch(browseros::kSparkleUrl)) {
+    std::string url = cmd->GetSwitchValueASCII(browseros::kSparkleUrl);
+    LOG(WARNING) << "Sparkle: Using override URL: " << url;
+    return base::SysUTF8ToNSString(url);
+  }
+
+  return GetArchitectureSpecificFeedURL();
+}
+
+- (void)updater:(SPUUpdater*)updater
+    didFinishLoadingAppcast:(SUAppcast*)appcast {
+  VLOG(1) << "Sparkle: Appcast loaded";
+}
+
+- (void)updater:(SPUUpdater*)updater
+    didFindValidUpdate:(SUAppcastItem*)item {
+  VLOG(1) << "Sparkle: Valid update found: "
+          << base::SysNSStringToUTF8(item.displayVersionString);
+}
+
+- (void)updaterDidNotFindUpdate:(SPUUpdater*)updater
+                          error:(NSError*)error {
+  // Already handled by user driver's showUpdateNotFoundWithError.
+}
+
+- (void)updater:(SPUUpdater*)updater
+    didAbortWithError:(NSError*)error {
+  if (error.code == SUNoUpdateError) {
+    // Not an actual error - just no update available.
+    return;
+  }
+
+  LOG(ERROR) << "Sparkle: Aborted with error: "
+             << base::SysNSStringToUTF8(error.localizedDescription);
+}
+
+#if !defined(OFFICIAL_BUILD)
+
+- (nullable NSString*)versionStringForUpdater:(SPUUpdater*)updater {
+  if (_spoofedVersion) {
+    return _spoofedVersion;
+  }
+  return nil;
+}
+
+- (BOOL)updater:(SPUUpdater*)updater
+    shouldAllowInsecureConnectionForHost:(NSString*)host
+                              isMainFeed:(BOOL)isMainFeed {
+  auto* cmd = base::CommandLine::ForCurrentProcess();
+  if (cmd && cmd->HasSwitch(browseros::kSparkleSkipSignature)) {
+    LOG(WARNING) << "Sparkle: Allowing insecure connection to "
+                 << base::SysNSStringToUTF8(host);
+    return YES;
+  }
+  return NO;
+}
+
+#pragma mark - Dry Run Simulation
+
+- (void)startDryRunSimulation {
+  LOG(WARNING) << "Sparkle: Starting DRY-RUN simulation";
+
+  [self setInternalStatus:SparkleStatusChecking];
+
+  dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 1 * NSEC_PER_SEC),
+                 dispatch_get_main_queue(), ^{
+                   [self setInternalStatus:SparkleStatusDownloading];
+                   [self simulateDownloadProgress];
+                 });
+}
+
+- (void)simulateDownloadProgress {
+  __block int progress = 0;
+  __weak SparkleGlue* weakSelf = self;
+
+  dispatch_source_t timer =
+      dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0,
+                             dispatch_get_main_queue());
+  dispatch_source_set_timer(timer, DISPATCH_TIME_NOW,
+                            100 * NSEC_PER_MSEC, 0);
+
+  dispatch_source_set_event_handler(timer, ^{
+    SparkleGlue* strongSelf = weakSelf;
+    if (!strongSelf) {
+      dispatch_source_cancel(timer);
+      return;
+    }
+
+    progress += 2;
+    SparkleProgress* p =
+        [[SparkleProgress alloc] initWithReceived:progress * 1024 * 1024
+                                            total:100 * 1024 * 1024];
+    [strongSelf notifyProgress:p];
+
+    if (progress >= 100) {
+      dispatch_source_cancel(timer);
+      [strongSelf setInternalStatus:SparkleStatusExtracting];
+
+      dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC),
+                     dispatch_get_main_queue(), ^{
+                       [strongSelf setInternalStatus:SparkleStatusReadyToInstall];
+                       NotifyUpgradeReady("999.0.0.0");
+                       LOG(WARNING) << "Sparkle: DRY-RUN complete";
+                     });
+    }
+  });
+
+  dispatch_resume(timer);
+}
+
+#endif  // !defined(OFFICIAL_BUILD)
+
+@end
+
+#pragma mark - C++ Namespace Functions
+
+namespace sparkle_glue {
+
+bool SparkleEnabled() {
+  return [SparkleGlue sharedSparkleGlue] != nil;
+}
+
+bool IsUpdateReady() {
+  SparkleGlue* glue = [SparkleGlue sharedSparkleGlue];
+  return glue != nil && glue.updateReady;
+}
+
+void InstallAndRelaunch() {
+  [[SparkleGlue sharedSparkleGlue] installAndRelaunch];
+}
+
+}  // namespace sparkle_glue
