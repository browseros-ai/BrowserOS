diff --git a/chrome/browser/extensions/api/browser_os/browser_os_attention_mac.mm b/chrome/browser/extensions/api/browser_os/browser_os_attention_mac.mm
new file mode 100644
index 0000000000000..417de1d07c874
--- /dev/null
+++ b/chrome/browser/extensions/api/browser_os/browser_os_attention_mac.mm
@@ -0,0 +1,44 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/extensions/api/browser_os/browser_os_attention_mac.h"
+
+#import <AppKit/AppKit.h>
+
+#include "base/strings/sys_string_conversions.h"
+
+namespace extensions::api::browser_os_attention {
+
+namespace {
+
+// The identifier of the bounce in progress, or 0. Only touched on the UI
+// thread, where extension functions run.
+NSInteger g_attention_request = 0;
+
+}  // namespace
+
+bool RequestAttention(bool critical) {
+  if (NSApp.active) {
+    return false;
+  }
+  CancelAttention();
+  g_attention_request = [NSApp
+      requestUserAttention:critical ? NSCriticalRequest
+                                    : NSInformationalRequest];
+  return true;
+}
+
+void CancelAttention() {
+  if (g_attention_request != 0) {
+    [NSApp cancelUserAttentionRequest:g_attention_request];
+    g_attention_request = 0;
+  }
+}
+
+void SetDockBadge(const std::string& text) {
+  NSApp.dockTile.badgeLabel =
+      text.empty() ? nil : base::SysUTF8ToNSString(text);
+}
+
+}  // namespace extensions::api::browser_os_attention
