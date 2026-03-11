diff --git a/chrome/browser/devtools/protocol/browser_handler_mac.h b/chrome/browser/devtools/protocol/browser_handler_mac.h
new file mode 100644
index 0000000000000..52e1b27fbfb0a
--- /dev/null
+++ b/chrome/browser/devtools/protocol/browser_handler_mac.h
@@ -0,0 +1,16 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_DEVTOOLS_PROTOCOL_BROWSER_HANDLER_MAC_H_
+#define CHROME_BROWSER_DEVTOOLS_PROTOCOL_BROWSER_HANDLER_MAC_H_
+
+class BrowserWindow;
+
+// Sets per-window headless mode on macOS. When headless, the window's
+// NSWindow swizzles AppKit methods to fake visibility while keeping the
+// compositor active. This bypasses constrainFrameRect:toScreen: which
+// otherwise clamps off-screen windows back on-screen.
+void SetWindowHeadless(BrowserWindow* window, bool headless);
+
+#endif  // CHROME_BROWSER_DEVTOOLS_PROTOCOL_BROWSER_HANDLER_MAC_H_
