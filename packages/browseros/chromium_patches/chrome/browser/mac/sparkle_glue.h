diff --git a/chrome/browser/mac/sparkle_glue.h b/chrome/browser/mac/sparkle_glue.h
new file mode 100644
index 0000000000000..c0b84c7873a18
--- /dev/null
+++ b/chrome/browser/mac/sparkle_glue.h
@@ -0,0 +1,51 @@
+// Copyright 2024 BlockBrowser Browser Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_MAC_SPARKLE_GLUE_H_
+#define CHROME_BROWSER_MAC_SPARKLE_GLUE_H_
+
+#import <Foundation/Foundation.h>
+
+// Forward declarations for C++ types in Objective-C context
+#ifdef __cplusplus
+#include "base/memory/weak_ptr.h"
+class SparkleVersionUpdater;
+#else
+typedef struct SparkleVersionUpdater SparkleVersionUpdater;
+#endif
+
+// Simple updater status for Sparkle integration
+enum UpdaterStatus {
+  kUpdaterStatusIdle = 0,
+  kUpdaterStatusChecking = 1,
+  kUpdaterStatusUpdateAvailable = 2,
+  kUpdaterStatusDownloading = 3,
+  kUpdaterStatusReadyToInstall = 4,
+  kUpdaterStatusError = 5,
+};
+
+@interface SparkleGlue : NSObject
+
++ (instancetype)sharedSparkleGlue;
+
+- (void)registerWithSparkle;
+- (void)checkForUpdates;
+- (BOOL)isUpdateCheckEnabled;
+
+// Set the version updater to receive status notifications
+#ifdef __cplusplus
+- (void)setVersionUpdater:(base::WeakPtr<SparkleVersionUpdater>)updater;
+#else
+- (void)setVersionUpdater:(void*)updater;
+#endif
+
+@end  // @interface SparkleGlue
+
+namespace sparkle_glue {
+
+bool SparkleEnabled();
+
+}  // namespace sparkle_glue
+
+#endif  // CHROME_BROWSER_MAC_SPARKLE_GLUE_H_
\ No newline at end of file
