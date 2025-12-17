diff --git a/chrome/browser/ui/webui/help/sparkle_version_updater_mac.mm b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.mm
new file mode 100644
index 0000000000000..47957c4108f40
--- /dev/null
+++ b/chrome/browser/ui/webui/help/sparkle_version_updater_mac.mm
@@ -0,0 +1,166 @@
+// Copyright 2024 BrowserOS Authors. All rights reserved.
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/webui/help/sparkle_version_updater_mac.h"
+
+#include "base/logging.h"
+#include "base/memory/raw_ptr_exclusion.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/sys_string_conversions.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/mac/sparkle_glue.h"
+
+#if !defined(__has_feature) || !__has_feature(objc_arc)
+#error "This file requires ARC support."
+#endif
+
+// Bridge class that implements SparkleObserver and forwards callbacks to the
+// C++ SparkleVersionUpdater.
+@interface SparkleVersionUpdaterBridge : NSObject <SparkleObserver>
+
+- (instancetype)initWithUpdater:(SparkleVersionUpdater*)updater;
+
+@end
+
+@implementation SparkleVersionUpdaterBridge {
+  // Not raw_ptr<> - prevent C++ template in Objective-C implementation block.
+  RAW_PTR_EXCLUSION SparkleVersionUpdater* _updater;
+}
+
+- (instancetype)initWithUpdater:(SparkleVersionUpdater*)updater {
+  if (self = [super init]) {
+    _updater = updater;
+  }
+  return self;
+}
+
+- (void)sparkleDidChangeStatus:(SparkleStatus)status {
+  if (_updater) {
+    _updater->OnStatusChanged(static_cast<int>(status));
+  }
+}
+
+- (void)sparkleDidUpdateProgress:(SparkleProgress*)progress {
+  if (_updater) {
+    _updater->OnProgressChanged(progress.percentage);
+  }
+}
+
+- (void)sparkleDidFailWithError:(NSString*)errorMessage {
+  if (_updater && errorMessage) {
+    _updater->OnError(base::SysNSStringToUTF8(errorMessage));
+  }
+}
+
+- (void)invalidate {
+  _updater = nullptr;
+}
+
+@end
+
+SparkleVersionUpdater::SparkleVersionUpdater() = default;
+
+SparkleVersionUpdater::~SparkleVersionUpdater() {
+  if (bridge_) {
+    [[SparkleGlue sharedSparkleGlue] removeObserver:bridge_];
+    [bridge_ invalidate];
+    bridge_ = nil;
+  }
+}
+
+void SparkleVersionUpdater::CheckForUpdate(StatusCallback status_callback,
+                                           PromoteCallback promote_callback) {
+  status_callback_ = std::move(status_callback);
+
+  SparkleGlue* sparkle = [SparkleGlue sharedSparkleGlue];
+  if (!sparkle) {
+    LOG(ERROR) << "SparkleVersionUpdater: Sparkle not available";
+    if (!status_callback_.is_null()) {
+      status_callback_.Run(FAILED, 0, false, false, std::string(), 0,
+                           u"Sparkle updater not available");
+    }
+    return;
+  }
+
+  // Create bridge if needed.
+  if (!bridge_) {
+    bridge_ = [[SparkleVersionUpdaterBridge alloc] initWithUpdater:this];
+    [sparkle addObserver:bridge_];
+  }
+
+  [sparkle checkForUpdates];
+}
+
+void SparkleVersionUpdater::PromoteUpdater() {
+  // Not applicable for Sparkle.
+}
+
+void SparkleVersionUpdater::OnStatusChanged(int status) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+
+  SparkleStatus sparkle_status = static_cast<SparkleStatus>(status);
+  Status update_status = CHECKING;
+  std::u16string message;
+
+  switch (sparkle_status) {
+    case SparkleStatusIdle:
+      return;
+
+    case SparkleStatusChecking:
+      update_status = CHECKING;
+      break;
+
+    case SparkleStatusDownloading:
+    case SparkleStatusExtracting:
+    case SparkleStatusInstalling:
+      update_status = UPDATING;
+      break;
+
+    case SparkleStatusReadyToInstall:
+      update_status = NEARLY_UPDATED;
+      break;
+
+    case SparkleStatusUpToDate:
+      update_status = UPDATED;
+      break;
+
+    case SparkleStatusError:
+      update_status = FAILED;
+      SparkleGlue* sparkle = [SparkleGlue sharedSparkleGlue];
+      if (sparkle && sparkle.lastErrorMessage) {
+        message = base::SysNSStringToUTF16(sparkle.lastErrorMessage);
+      }
+      break;
+  }
+
+  status_callback_.Run(update_status, 0, false, false, std::string(), 0,
+                       message);
+}
+
+void SparkleVersionUpdater::OnProgressChanged(int percentage) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+
+  VLOG(2) << "SparkleVersionUpdater: Progress " << percentage << "%";
+
+  std::u16string message =
+      u"Downloading update: " + base::NumberToString16(percentage) + u"%";
+
+  status_callback_.Run(UPDATING, percentage, false, false, std::string(), 0,
+                       message);
+}
+
+void SparkleVersionUpdater::OnError(const std::string& message) {
+  if (status_callback_.is_null()) {
+    return;
+  }
+
+  LOG(ERROR) << "SparkleVersionUpdater: Error - " << message;
+
+  status_callback_.Run(FAILED, 0, false, false, std::string(), 0,
+                       base::UTF8ToUTF16(message));
+}
