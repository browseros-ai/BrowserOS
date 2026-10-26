diff --git a/chrome/browser/extensions/api/browser_os/browser_os_attention_mac.h b/chrome/browser/extensions/api/browser_os/browser_os_attention_mac.h
new file mode 100644
index 0000000000000..d4aa3d7736e6d
--- /dev/null
+++ b/chrome/browser/extensions/api/browser_os/browser_os_attention_mac.h
@@ -0,0 +1,25 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_ATTENTION_MAC_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_ATTENTION_MAC_H_
+
+#include <string>
+
+namespace extensions::api::browser_os_attention {
+
+// Bounces the dock icon: once, or until BrowserOS is the active app when
+// |critical|. Returns false when BrowserOS is already active, where macOS
+// shows nothing. A new request replaces the previous one.
+bool RequestAttention(bool critical);
+
+// Stops the bounce started by RequestAttention, if it is still going.
+void CancelAttention();
+
+// Sets the dock icon's badge; an empty |text| removes it.
+void SetDockBadge(const std::string& text);
+
+}  // namespace extensions::api::browser_os_attention
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_ATTENTION_MAC_H_
