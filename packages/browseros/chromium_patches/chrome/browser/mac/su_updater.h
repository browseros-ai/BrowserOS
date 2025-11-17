diff --git a/chrome/browser/mac/su_updater.h b/chrome/browser/mac/su_updater.h
new file mode 100644
index 0000000000000..f857acdfbfa4d
--- /dev/null
+++ b/chrome/browser/mac/su_updater.h
@@ -0,0 +1,42 @@
+// Copyright 2024 BlockBrowser Browser Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_MAC_SU_UPDATER_H_
+#define CHROME_BROWSER_MAC_SU_UPDATER_H_
+
+#import <Foundation/Foundation.h>
+
+// Forward declarations for Sparkle framework classes
+@class SUAppcast;
+@class SUAppcastItem;
+
+@interface SUUpdater : NSObject
+
++ (SUUpdater*)sharedUpdater;
+
+- (void)setDelegate:(id)delegate;
+- (void)setAutomaticallyChecksForUpdates:(BOOL)enable;
+- (void)setAutomaticallyDownloadsUpdates:(BOOL)enable;
+- (void)setUpdateCheckInterval:(NSTimeInterval)interval;
+- (void)checkForUpdatesInBackground;
+- (void)checkForUpdates:(id)sender;
+
+@property BOOL automaticallyDownloadsUpdates;
+
+@end
+
+// SUUpdaterDelegate protocol (partial)
+@protocol SUUpdaterDelegate <NSObject>
+@optional
+- (NSString*)feedURLStringForUpdater:(SUUpdater*)updater;
+- (void)updater:(SUUpdater*)updater didFinishLoadingAppcast:(SUAppcast*)appcast;
+- (void)updater:(SUUpdater*)updater didFindValidUpdate:(SUAppcastItem*)item;
+- (void)updaterDidNotFindUpdate:(SUUpdater*)updater;
+- (void)updater:(SUUpdater*)updater willInstallUpdate:(SUAppcastItem*)item;
+- (void)updater:(SUUpdater*)updater didAbortWithError:(NSError*)error;
+- (void)updater:(SUUpdater*)updater userDidCancelDownload:(SUAppcastItem*)item;
+- (void)downloaderDidDownloadUpdate:(SUAppcastItem*)item withProgress:(double)progress;
+@end
+
+#endif  // CHROME_BROWSER_MAC_SU_UPDATER_H_
\ No newline at end of file
