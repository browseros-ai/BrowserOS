diff --git a/chrome/browser/mac/sparkle_glue.h b/chrome/browser/mac/sparkle_glue.h
new file mode 100644
index 0000000000000..ad8ef816c7cd4
--- /dev/null
+++ b/chrome/browser/mac/sparkle_glue.h
@@ -0,0 +1,90 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_MAC_SPARKLE_GLUE_H_
+#define CHROME_BROWSER_MAC_SPARKLE_GLUE_H_
+
+#import <Foundation/Foundation.h>
+
+NS_ASSUME_NONNULL_BEGIN
+
+// Sparkle updater status.
+typedef NS_ENUM(NSInteger, SparkleStatus) {
+  SparkleStatusIdle = 0,
+  SparkleStatusChecking,
+  SparkleStatusDownloading,
+  SparkleStatusExtracting,
+  SparkleStatusReadyToInstall,
+  SparkleStatusInstalling,
+  SparkleStatusUpToDate,
+  SparkleStatusError,
+};
+
+// Progress information for download/extraction operations.
+@interface SparkleProgress : NSObject
+
+@property(nonatomic, readonly) double fraction;
+@property(nonatomic, readonly) uint64_t bytesReceived;
+@property(nonatomic, readonly) uint64_t bytesTotal;
+@property(nonatomic, readonly) int percentage;
+
+- (instancetype)initWithReceived:(uint64_t)received total:(uint64_t)total;
+
+@end
+
+// Protocol for observing Sparkle update status changes.
+@protocol SparkleObserver <NSObject>
+
+- (void)sparkleDidChangeStatus:(SparkleStatus)status;
+- (void)sparkleDidUpdateProgress:(SparkleProgress*)progress;
+
+@optional
+- (void)sparkleDidFailWithError:(NSString*)errorMessage;
+
+@end
+
+// Main interface for Sparkle integration.
+// Thread-safety: All methods must be called on the main thread.
+//
+// Version comparison is entirely plist-driven: Sparkle compares the
+// appcast's sparkle:version against the outer bundle's CFBundleVersion,
+// which the release pipeline stamps with "10000.<BrowserOS version>"
+// before signing (Context.get_sparkle_version in the BrowserOS repo).
+// Windows mirrors the same string via winsparkle_glue.cc.
+@interface SparkleGlue : NSObject
+
++ (nullable instancetype)sharedSparkleGlue;
+
+// Current status.
+@property(nonatomic, readonly) SparkleStatus status;
+@property(nonatomic, readonly) BOOL updateReady;
+@property(nonatomic, readonly, nullable) NSString* lastErrorMessage;
+
+// Actions.
+- (void)checkForUpdates;
+- (void)installAndRelaunch;
+
+// Observer management. Observers are held weakly.
+- (void)addObserver:(id<SparkleObserver>)observer;
+- (void)removeObserver:(id<SparkleObserver>)observer;
+
+@end
+
+NS_ASSUME_NONNULL_END
+
+// C++ functions for non-ObjC code.
+namespace sparkle_glue {
+
+// Returns true if Sparkle is enabled and initialized.
+bool SparkleEnabled();
+
+// Returns true if an update has been downloaded and is ready to install.
+bool IsUpdateReady();
+
+// Triggers installation of downloaded update and relaunches the app.
+void InstallAndRelaunch();
+
+}  // namespace sparkle_glue
+
+#endif  // CHROME_BROWSER_MAC_SPARKLE_GLUE_H_
