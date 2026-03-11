diff --git a/chrome/browser/devtools/protocol/browser_handler_mac.mm b/chrome/browser/devtools/protocol/browser_handler_mac.mm
new file mode 100644
index 0000000000000..cd806bae50afd
--- /dev/null
+++ b/chrome/browser/devtools/protocol/browser_handler_mac.mm
@@ -0,0 +1,19 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/devtools/protocol/browser_handler_mac.h"
+
+#import <Cocoa/Cocoa.h>
+
+#include "chrome/browser/ui/browser_window.h"
+#include "components/remote_cocoa/app_shim/native_widget_mac_nswindow.h"
+#include "ui/gfx/native_ui_types.h"
+
+void SetWindowHeadless(BrowserWindow* window, bool headless) {
+  NSWindow* ns_window = window->GetNativeWindow().GetNativeNSWindow();
+  if (auto* native_window =
+          base::apple::ObjCCast<NativeWidgetMacNSWindow>(ns_window)) {
+    [native_window setIsHeadless:headless];
+  }
+}
