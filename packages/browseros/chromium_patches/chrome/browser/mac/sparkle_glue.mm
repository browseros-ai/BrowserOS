diff --git a/chrome/browser/mac/sparkle_glue.mm b/chrome/browser/mac/sparkle_glue.mm
new file mode 100644
index 0000000000000..cf39a1f49a55a
--- /dev/null
+++ b/chrome/browser/mac/sparkle_glue.mm
@@ -0,0 +1,580 @@
+// Copyright 2024 BlockBrowser Browser Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#import "chrome/browser/mac/sparkle_glue.h"
+
+#include <sys/mount.h>
+#include <sys/stat.h>
+
+#include "base/apple/bundle_locations.h"
+#include "base/apple/foundation_util.h"
+#include "base/apple/scoped_nsautorelease_pool.h"
+#include "base/command_line.h"
+#include "base/logging.h"
+#include "base/memory/weak_ptr.h"
+#include "base/strings/sys_string_conversions.h"
+#include "base/system/sys_info.h"
+#include "chrome/browser/mac/su_updater.h"
+#include "chrome/browser/ui/webui/help/sparkle_version_updater_mac.h"
+#include "chrome/common/chrome_switches.h"
+
+#if !defined(__has_feature) || !__has_feature(objc_arc)
+#error "This file requires ARC support."
+#endif
+
+namespace {
+
+// Check for updates every 30 minutes
+constexpr NSTimeInterval kUpdateCheckIntervalInSec = 30 * 60;
+
+
+// Default update feed URL - architecture specific
+NSString* GetUpdateFeedURL() {
+  @try {
+    // You can override with command line flag: --update-feed-url=<url>
+    auto* command_line = base::CommandLine::ForCurrentProcess();
+    if (command_line && command_line->HasSwitch("update-feed-url")) {
+      std::string override_url = command_line->GetSwitchValueASCII("update-feed-url");
+      LOG(INFO) << "SparkleGlue: Using override update URL: " << override_url;
+      return base::SysUTF8ToNSString(override_url);
+    }
+
+    // Use default appcast.xml for ARM64, add suffix for x86_64
+    std::string url;
+    if (base::SysInfo::OperatingSystemArchitecture() == "x86_64") {
+      url = "https://cdn.browseros.com/appcast-x86_64.xml";
+      LOG(INFO) << "SparkleGlue: System architecture: x86_64, using appcast URL: " << url;
+    } else {
+      url = "https://cdn.browseros.com/appcast.xml";
+      LOG(INFO) << "SparkleGlue: System architecture: " << base::SysInfo::OperatingSystemArchitecture() 
+                << " (ARM64), using default appcast URL: " << url;
+    }
+    return base::SysUTF8ToNSString(url);
+  } @catch (NSException* exception) {
+    LOG(ERROR) << "SparkleGlue: Exception in GetUpdateFeedURL, falling back to default";
+    // Fallback to default (ARM64)
+    return @"https://cdn.browseros.com/appcast.xml";
+  }
+}
+
+
+}  // namespace
+
+@implementation SparkleGlue {
+  SUUpdater* __strong _updater;
+  BOOL _registered;
+  NSString* __strong _appPath;
+  base::WeakPtr<SparkleVersionUpdater> _versionUpdater;  // Weak reference
+  BOOL _initializationAttempted;
+}
+
++ (instancetype)sharedSparkleGlue {
+  static SparkleGlue* shared = nil;
+  static dispatch_once_t onceToken;
+
+  dispatch_once(&onceToken, ^{
+    @try {
+      LOG(INFO) << "SparkleGlue: Creating shared instance";
+      
+      // Check if updates are disabled via command line
+      auto* command_line = base::CommandLine::ForCurrentProcess();
+      if (command_line && command_line->HasSwitch("disable-updates")) {
+        LOG(INFO) << "SparkleGlue: Updates disabled via command line";
+        return;
+      }
+
+      shared = [[SparkleGlue alloc] init];
+      LOG(INFO) << "SparkleGlue: Shared instance created successfully";
+    } @catch (NSException* exception) {
+      LOG(ERROR) << "SparkleGlue: Exception creating shared instance: " 
+                 << base::SysNSStringToUTF8([exception description]);
+      shared = nil;
+    } @catch (...) {
+      LOG(ERROR) << "SparkleGlue: C++ exception creating shared instance";
+      shared = nil;
+    }
+  });
+
+  return shared;
+}
+
+- (instancetype)init {
+  @try {
+    if (self = [super init]) {
+      _registered = NO;
+      _initializationAttempted = NO;
+      _appPath = [base::apple::OuterBundle().bundlePath copy];
+      
+      LOG(INFO) << "SparkleGlue: Init started, app path: " << base::SysNSStringToUTF8(_appPath);
+      
+      // Defer framework loading to main queue with delay
+      // This ensures all Chrome subsystems are initialized
+      dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC),
+                     dispatch_get_main_queue(), ^{
+        LOG(INFO) << "SparkleGlue: Attempting deferred initialization";
+        [self attemptSparkleInitialization];
+      });
+      
+      return self;
+    }
+  } @catch (NSException* exception) {
+    LOG(ERROR) << "SparkleGlue: Exception in init: " << base::SysNSStringToUTF8([exception description]);
+    return nil;
+  }
+  return nil;
+}
+
+- (void)attemptSparkleInitialization {
+  @try {
+    if (_initializationAttempted) {
+      LOG(INFO) << "SparkleGlue: Initialization already attempted";
+      return;
+    }
+    _initializationAttempted = YES;
+    
+    LOG(INFO) << "SparkleGlue: Beginning Sparkle initialization";
+    
+    if ([self loadSparkleFramework]) {
+      LOG(INFO) << "SparkleGlue: Framework loaded successfully, registering with Sparkle";
+      [self registerWithSparkle];
+    } else {
+      LOG(ERROR) << "SparkleGlue: Failed to load Sparkle framework";
+    }
+  } @catch (NSException* exception) {
+    LOG(ERROR) << "SparkleGlue: Exception in attemptSparkleInitialization: " 
+               << base::SysNSStringToUTF8([exception description]);
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: C++ exception in attemptSparkleInitialization";
+  }
+}
+
+- (BOOL)loadSparkleFramework {
+  @try {
+    base::apple::ScopedNSAutoreleasePool pool;
+
+    LOG(INFO) << "SparkleGlue: Loading Sparkle framework";
+    
+    // Check if running from read-only filesystem (e.g., DMG)
+    if ([self isOnReadOnlyFilesystem]) {
+      LOG(INFO) << "SparkleGlue: Running from read-only filesystem, skipping Sparkle";
+      return NO;
+    }
+
+    // Try multiple paths for the Sparkle framework
+    NSArray<NSString*>* searchPaths = @[
+      // Path 1: Inside the Chromium Framework bundle (where it's actually bundled)
+      [[base::apple::FrameworkBundle() privateFrameworksPath]
+          stringByAppendingPathComponent:@"Sparkle.framework"],
+      
+      // Path 2: In the main app's Frameworks directory
+      [[base::apple::OuterBundle() privateFrameworksPath]
+          stringByAppendingPathComponent:@"Sparkle.framework"],
+      
+      // Path 3: Relative to the framework bundle
+      [[[base::apple::FrameworkBundle() bundlePath] 
+          stringByAppendingPathComponent:@"Frameworks"]
+          stringByAppendingPathComponent:@"Sparkle.framework"]
+    ];
+
+    NSBundle* sparkle_bundle = nil;
+    
+    LOG(INFO) << "SparkleGlue: Searching for Sparkle framework...";
+    for (NSString* path in searchPaths) {
+      LOG(INFO) << "SparkleGlue: Checking path: " << base::SysNSStringToUTF8(path);
+      if ([[NSFileManager defaultManager] fileExistsAtPath:path]) {
+        LOG(INFO) << "SparkleGlue: Found framework at path: " << base::SysNSStringToUTF8(path);
+        sparkle_bundle = [NSBundle bundleWithPath:path];
+        if (sparkle_bundle) {
+          LOG(INFO) << "SparkleGlue: Successfully created NSBundle for Sparkle";
+          break;
+        } else {
+          LOG(ERROR) << "SparkleGlue: Failed to create NSBundle for path: " << base::SysNSStringToUTF8(path);
+        }
+      }
+    }
+
+    if (!sparkle_bundle) {
+      LOG(ERROR) << "SparkleGlue: Could not find Sparkle framework in any search path";
+      return NO;
+    }
+
+    // Check if already loaded
+    if (![sparkle_bundle isLoaded]) {
+      LOG(INFO) << "SparkleGlue: Loading Sparkle bundle...";
+      NSError* load_error = nil;
+      if (![sparkle_bundle loadAndReturnError:&load_error]) {
+        LOG(ERROR) << "SparkleGlue: Failed to load Sparkle bundle: " 
+                   << base::SysNSStringToUTF8([load_error localizedDescription]);
+        return NO;
+      }
+      LOG(INFO) << "SparkleGlue: Sparkle bundle loaded successfully";
+    } else {
+      LOG(INFO) << "SparkleGlue: Sparkle bundle already loaded";
+    }
+
+    // Get SUUpdater class and create shared instance
+    Class updater_class = [sparkle_bundle classNamed:@"SUUpdater"];
+    if (!updater_class) {
+      LOG(ERROR) << "SparkleGlue: Could not find SUUpdater class in Sparkle framework";
+      return NO;
+    }
+    LOG(INFO) << "SparkleGlue: Found SUUpdater class"; 
+
+    // Use performSelector to avoid direct class dependencies
+    SEL sharedUpdaterSelector = NSSelectorFromString(@"sharedUpdater");
+    if (![updater_class respondsToSelector:sharedUpdaterSelector]) {
+      LOG(ERROR) << "SparkleGlue: SUUpdater class does not respond to sharedUpdater selector";
+      return NO;
+    }
+    LOG(INFO) << "SparkleGlue: SUUpdater responds to sharedUpdater selector";
+    
+#pragma clang diagnostic push
+#pragma clang diagnostic ignored "-Warc-performSelector-leaks"
+    _updater = [updater_class performSelector:sharedUpdaterSelector];
+#pragma clang diagnostic pop
+    
+    if (!_updater) {
+      LOG(ERROR) << "SparkleGlue: Failed to get shared SUUpdater instance";
+      return NO;
+    }
+
+    LOG(INFO) << "SparkleGlue: Successfully obtained SUUpdater instance";
+    return YES;
+    
+  } @catch (NSException* exception) {
+    LOG(ERROR) << "SparkleGlue: Exception in loadSparkleFramework: " 
+               << base::SysNSStringToUTF8([exception description]);
+    return NO;
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: C++ exception in loadSparkleFramework";
+    return NO;
+  }
+}
+
+- (void)registerWithSparkle {
+  @try {
+    if (_registered || !_updater) {
+      LOG(INFO) << "SparkleGlue: Already registered or no updater available";
+      return;
+    }
+
+    LOG(INFO) << "SparkleGlue: Beginning Sparkle registration";
+    _registered = YES;
+
+    // Configure updater using performSelector to avoid direct dependencies
+    SEL setDelegateSelector = NSSelectorFromString(@"setDelegate:");
+    if ([_updater respondsToSelector:setDelegateSelector]) {
+      LOG(INFO) << "SparkleGlue: Setting delegate";
+#pragma clang diagnostic push
+#pragma clang diagnostic ignored "-Warc-performSelector-leaks"
+      [_updater performSelector:setDelegateSelector withObject:self];
+#pragma clang diagnostic pop
+    } else {
+      LOG(ERROR) << "SparkleGlue: SUUpdater does not respond to setDelegate:";
+    }
+
+    // Set update check interval
+    SEL setIntervalSelector = NSSelectorFromString(@"setUpdateCheckInterval:");
+    if ([_updater respondsToSelector:setIntervalSelector]) {
+      LOG(INFO) << "SparkleGlue: Setting update check interval to " << kUpdateCheckIntervalInSec << " seconds";
+      NSMethodSignature* sig = [_updater methodSignatureForSelector:setIntervalSelector];
+      NSInvocation* invocation = [NSInvocation invocationWithMethodSignature:sig];
+      [invocation setTarget:_updater];
+      [invocation setSelector:setIntervalSelector];
+      NSTimeInterval interval = kUpdateCheckIntervalInSec;
+      [invocation setArgument:&interval atIndex:2];
+      [invocation invoke];
+    } else {
+      LOG(WARNING) << "SparkleGlue: SUUpdater does not respond to setUpdateCheckInterval:";
+    }
+
+    // Set automatic checks
+    SEL setAutoCheckSelector = NSSelectorFromString(@"setAutomaticallyChecksForUpdates:");
+    if ([_updater respondsToSelector:setAutoCheckSelector]) {
+      LOG(INFO) << "SparkleGlue: Enabling automatic update checks";
+      NSMethodSignature* sig = [_updater methodSignatureForSelector:setAutoCheckSelector];
+      NSInvocation* invocation = [NSInvocation invocationWithMethodSignature:sig];
+      [invocation setTarget:_updater];
+      [invocation setSelector:setAutoCheckSelector];
+      BOOL value = YES;
+      [invocation setArgument:&value atIndex:2];
+      [invocation invoke];
+    } else {
+      LOG(WARNING) << "SparkleGlue: SUUpdater does not respond to setAutomaticallyChecksForUpdates:";
+    }
+
+    // Set automatic downloads
+    SEL setAutoDownloadSelector = NSSelectorFromString(@"setAutomaticallyDownloadsUpdates:");
+    if ([_updater respondsToSelector:setAutoDownloadSelector]) {
+      LOG(INFO) << "SparkleGlue: Enabling automatic downloads";
+      NSMethodSignature* sig = [_updater methodSignatureForSelector:setAutoDownloadSelector];
+      NSInvocation* invocation = [NSInvocation invocationWithMethodSignature:sig];
+      [invocation setTarget:_updater];
+      [invocation setSelector:setAutoDownloadSelector];
+      BOOL value = YES;
+      [invocation setArgument:&value atIndex:2];
+      [invocation invoke];
+    } else {
+      LOG(WARNING) << "SparkleGlue: SUUpdater does not respond to setAutomaticallyDownloadsUpdates:";
+    }
+
+    // Set feed URL
+    SEL setFeedURLSelector = NSSelectorFromString(@"setFeedURL:");
+    if ([_updater respondsToSelector:setFeedURLSelector]) {
+      NSString* feedURLString = GetUpdateFeedURL();
+      LOG(INFO) << "SparkleGlue: Setting feed URL to: " << base::SysNSStringToUTF8(feedURLString);
+      if (feedURLString) {
+        NSURL* feedURL = [NSURL URLWithString:feedURLString];
+        if (feedURL) {
+#pragma clang diagnostic push
+#pragma clang diagnostic ignored "-Warc-performSelector-leaks"
+          [_updater performSelector:setFeedURLSelector withObject:feedURL];
+#pragma clang diagnostic pop
+          LOG(INFO) << "SparkleGlue: Feed URL set successfully";
+        } else {
+          LOG(ERROR) << "SparkleGlue: Failed to create NSURL from feed string";
+        }
+      } else {
+        LOG(ERROR) << "SparkleGlue: Feed URL string is nil";
+      }
+    } else {
+      LOG(ERROR) << "SparkleGlue: SUUpdater does not respond to setFeedURL:";
+    }
+
+    LOG(INFO) << "SparkleGlue: Registration complete";
+
+  } @catch (NSException* exception) {
+    LOG(ERROR) << "SparkleGlue: Exception in registerWithSparkle: " 
+               << base::SysNSStringToUTF8([exception description]);
+    _registered = NO;
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: C++ exception in registerWithSparkle";
+    _registered = NO;
+  }
+}
+
+- (void)checkForUpdates {
+  @try {
+    if (!_registered || !_updater) {
+      LOG(WARNING) << "SparkleGlue: Cannot check for updates - not registered or no updater";
+      return;
+    }
+
+    LOG(INFO) << "SparkleGlue: Starting update check";
+    
+    SEL checkSelector = NSSelectorFromString(@"checkForUpdatesInBackground");
+    if ([_updater respondsToSelector:checkSelector]) {
+      LOG(INFO) << "SparkleGlue: Calling checkForUpdatesInBackground";
+#pragma clang diagnostic push
+#pragma clang diagnostic ignored "-Warc-performSelector-leaks"
+      [_updater performSelector:checkSelector];
+#pragma clang diagnostic pop
+    } else {
+      LOG(ERROR) << "SparkleGlue: SUUpdater does not respond to checkForUpdatesInBackground";
+    }
+  } @catch (NSException* exception) {
+    LOG(ERROR) << "SparkleGlue: Exception in checkForUpdates: " 
+               << base::SysNSStringToUTF8([exception description]);
+  }
+}
+
+- (BOOL)isUpdateCheckEnabled {
+  return _registered && _updater != nil;
+}
+
+- (BOOL)isOnReadOnlyFilesystem {
+  @try {
+    const char* appPathC = _appPath.fileSystemRepresentation;
+    struct statfs statfsBuf;
+
+    if (statfs(appPathC, &statfsBuf) != 0) {
+      return NO;
+    }
+
+    return (statfsBuf.f_flags & MNT_RDONLY) != 0;
+  } @catch (NSException* exception) {
+    return NO;
+  }
+}
+
+- (void)setVersionUpdater:(base::WeakPtr<SparkleVersionUpdater>)updater {
+  @try {
+    _versionUpdater = updater;
+  } @catch (NSException* exception) {
+    // Ignore
+  }
+}
+
+#pragma mark - SUUpdaterDelegate
+
+- (NSString*)feedURLStringForUpdater:(SUUpdater*)updater {
+  @try {
+    return GetUpdateFeedURL();
+  } @catch (NSException* exception) {
+    // Fallback to default appcast
+    return @"https://cdn.browseros.com/appcast.xml";
+  }
+}
+
+- (void)updater:(SUUpdater*)updater didFinishLoadingAppcast:(SUAppcast*)appcast {
+  @try {
+    LOG(INFO) << "SparkleGlue: didFinishLoadingAppcast - appcast loaded successfully";
+    
+    // Notify version updater that we're still checking
+    if (auto* versionUpdater = _versionUpdater.get()) {
+      versionUpdater->OnSparkleStatusChange(kSparkleStatusChecking);
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in didFinishLoadingAppcast";
+  }
+}
+
+- (void)updater:(SUUpdater*)updater didFindValidUpdate:(SUAppcastItem*)item {
+  @try {
+    LOG(INFO) << "SparkleGlue: didFindValidUpdate - update available";
+    
+    if (auto* versionUpdater = _versionUpdater.get()) {
+      versionUpdater->OnSparkleStatusChange(kSparkleStatusUpdateFound);
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in didFindValidUpdate";
+  }
+}
+
+- (void)updaterDidNotFindUpdate:(SUUpdater*)updater {
+  @try {
+    LOG(INFO) << "SparkleGlue: updaterDidNotFindUpdate - no update available";
+    if (auto* versionUpdater = _versionUpdater.get()) {
+      versionUpdater->OnSparkleStatusChange(kSparkleStatusNoUpdate);
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in updaterDidNotFindUpdate";
+  }
+}
+
+- (void)updater:(SUUpdater*)updater willInstallUpdate:(SUAppcastItem*)item {
+  @try {
+    LOG(INFO) << "SparkleGlue: willInstallUpdate called";
+    
+    if (auto* versionUpdater = _versionUpdater.get()) {
+      versionUpdater->OnSparkleStatusChange(kSparkleStatusReadyToInstall);
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in willInstallUpdate";
+  }
+}
+
+- (void)updater:(SUUpdater*)updater didAbortWithError:(NSError*)error {
+  @try {
+    // Log detailed error information
+    NSString* errorDesc = [error localizedDescription];
+    NSString* errorDomain = [error domain];
+    NSInteger errorCode = [error code];
+    NSDictionary* userInfo = [error userInfo];
+    
+    LOG(ERROR) << "SparkleGlue: didAbortWithError called";
+    LOG(ERROR) << "  Error domain: " << base::SysNSStringToUTF8(errorDomain);
+    LOG(ERROR) << "  Error code: " << errorCode;
+    LOG(ERROR) << "  Error description: " << base::SysNSStringToUTF8(errorDesc);
+    
+    // Log additional error details from userInfo
+    if (userInfo) {
+      for (NSString* key in userInfo) {
+        id value = userInfo[key];
+        if ([value isKindOfClass:[NSString class]]) {
+          LOG(ERROR) << "  UserInfo[" << base::SysNSStringToUTF8(key) << "]: " 
+                     << base::SysNSStringToUTF8((NSString*)value);
+        }
+      }
+    }
+    
+    // Check for specific signature verification errors
+    if ([errorDomain isEqualToString:@"SUSparkleErrorDomain"]) {
+      LOG(ERROR) << "SparkleGlue: This is a Sparkle-specific error";
+      
+      // Common Sparkle error codes
+      switch (errorCode) {
+        case 3000:  // SUSignatureError
+          LOG(ERROR) << "SparkleGlue: Signature verification failed (SUSignatureError)";
+          break;
+        case 3001:  // SUAuthenticationError  
+          LOG(ERROR) << "SparkleGlue: Authentication failed (SUAuthenticationError)";
+          break;
+        case 3002:  // SUMissingUpdateError
+          LOG(ERROR) << "SparkleGlue: Missing update error (SUMissingUpdateError)";
+          break;
+        case 3003:  // SUMissingInstallerError
+          LOG(ERROR) << "SparkleGlue: Missing installer error (SUMissingInstallerError)";
+          break;
+        case 3004:  // SURelaunchError
+          LOG(ERROR) << "SparkleGlue: Relaunch error (SURelaunchError)";
+          break;
+        case 3005:  // SUInstallationError
+          LOG(ERROR) << "SparkleGlue: Installation error (SUInstallationError)";
+          break;
+        case 3006:  // SUDowngradeError
+          LOG(ERROR) << "SparkleGlue: Downgrade error (SUDowngradeError)";
+          break;
+        default:
+          LOG(ERROR) << "SparkleGlue: Unknown Sparkle error code: " << errorCode;
+      }
+    }
+    
+    // Check if this is actually an error or just "no update needed"
+    if ([errorDesc containsString:@"up to date"] || 
+        [errorDesc containsString:@"You're up to date"]) {
+      LOG(INFO) << "SparkleGlue: Not really an error - no update needed";
+      if (auto* versionUpdater = _versionUpdater.get()) {
+        versionUpdater->OnSparkleStatusChange(kSparkleStatusNoUpdate);
+      }
+    } else {
+      // This is a real error
+      // Notify the version updater
+      if (auto* versionUpdater = _versionUpdater.get()) {
+        versionUpdater->OnSparkleStatusChange(kSparkleStatusError, 
+                                               base::SysNSStringToUTF8(errorDesc));
+      }
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in didAbortWithError";
+  }
+}
+
+- (void)downloaderDidDownloadUpdate:(SUAppcastItem*)item withProgress:(double)progress {
+  @try {
+    LOG(INFO) << "SparkleGlue: Download progress: " << (progress * 100) << "%";
+    if (auto* versionUpdater = _versionUpdater.get()) {
+      versionUpdater->OnDownloadProgress(progress);
+      // Also notify that we're in downloading state
+      versionUpdater->OnSparkleStatusChange(kSparkleStatusDownloading);
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in downloaderDidDownloadUpdate";
+  }
+}
+
+- (void)updater:(SUUpdater*)updater userDidCancelDownload:(SUAppcastItem*)item {
+  @try {
+    LOG(INFO) << "SparkleGlue: User cancelled download";
+    if (auto* versionUpdater = _versionUpdater.get()) {
+      versionUpdater->OnSparkleStatusChange(kSparkleStatusError, "Download cancelled by user");
+    }
+  } @catch (...) {
+    LOG(ERROR) << "SparkleGlue: Exception in userDidCancelDownload";
+  }
+}
+
+@end
+
+namespace sparkle_glue {
+
+bool SparkleEnabled() {
+  @try {
+    return [SparkleGlue sharedSparkleGlue] != nil;
+  } @catch (...) {
+    return false;
+  }
+}
+
+}  // namespace sparkle_glue
