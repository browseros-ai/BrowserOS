diff --git a/chrome/browser/extensions/api/blockbrowser/blockbrowser_api.h b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api.h
new file mode 100644
index 0000000000000..e4b1c5f821342
--- /dev/null
+++ b/chrome/browser/extensions/api/blockbrowser/blockbrowser_api.h
@@ -0,0 +1,331 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
+
+#include <cstdint>
+
+#include "base/memory/raw_ptr.h"
+#include "base/values.h"
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_api_utils.h"
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_content_processor.h"
+#include "chrome/browser/extensions/api/blockbrowser/blockbrowser_snapshot_processor.h"
+#include "extensions/browser/extension_function.h"
+#include "third_party/skia/include/core/SkBitmap.h"
+
+namespace content {
+class WebContents;
+}
+
+namespace ui {
+struct AXTreeUpdate;
+}
+
+namespace extensions {
+namespace api {
+
+
+class BlockBrowserGetAccessibilityTreeFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getAccessibilityTree",
+                             BROWSER_OS_GETACCESSIBILITYTREE)
+
+  BlockBrowserGetAccessibilityTreeFunction() = default;
+
+ protected:
+  ~BlockBrowserGetAccessibilityTreeFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+
+ private:
+  void OnAccessibilityTreeReceived(ui::AXTreeUpdate& tree_update);
+};
+
+class BlockBrowserGetInteractiveSnapshotFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getInteractiveSnapshot",
+                             BROWSER_OS_GETINTERACTIVESNAPSHOT)
+
+  BlockBrowserGetInteractiveSnapshotFunction();
+
+ protected:
+  ~BlockBrowserGetInteractiveSnapshotFunction() override;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+
+ private:
+  void OnAccessibilityTreeReceived(ui::AXTreeUpdate& tree_update);
+  void OnSnapshotProcessed(SnapshotProcessingResult result);
+  
+  // Counter for snapshot IDs
+  static uint32_t next_snapshot_id_;
+  
+  // Tab ID for storing mappings
+  int tab_id_ = -1;
+  
+  // Web contents for processing and drawing
+  raw_ptr<content::WebContents> web_contents_ = nullptr;
+};
+
+class BlockBrowserClickFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.click", BROWSER_OS_CLICK)
+
+  BlockBrowserClickFunction() = default;
+
+ protected:
+  ~BlockBrowserClickFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserInputTextFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.inputText", BROWSER_OS_INPUTTEXT)
+
+  BlockBrowserInputTextFunction() = default;
+
+ protected:
+  ~BlockBrowserInputTextFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserClearFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.clear", BROWSER_OS_CLEAR)
+
+  BlockBrowserClearFunction() = default;
+
+ protected:
+  ~BlockBrowserClearFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserGetPageLoadStatusFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getPageLoadStatus", 
+                             BROWSER_OS_GETPAGELOADSTATUS)
+
+  BlockBrowserGetPageLoadStatusFunction() = default;
+
+ protected:
+  ~BlockBrowserGetPageLoadStatusFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserScrollUpFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.scrollUp", BROWSER_OS_SCROLLUP)
+
+  BlockBrowserScrollUpFunction() = default;
+
+ protected:
+  ~BlockBrowserScrollUpFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserScrollDownFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.scrollDown", BROWSER_OS_SCROLLDOWN)
+
+  BlockBrowserScrollDownFunction() = default;
+
+ protected:
+  ~BlockBrowserScrollDownFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserScrollToNodeFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.scrollToNode", BROWSER_OS_SCROLLTONODE)
+
+  BlockBrowserScrollToNodeFunction() = default;
+
+ protected:
+  ~BlockBrowserScrollToNodeFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserSendKeysFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.sendKeys", BROWSER_OS_SENDKEYS)
+
+  BlockBrowserSendKeysFunction() = default;
+
+ protected:
+  ~BlockBrowserSendKeysFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserCaptureScreenshotFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.captureScreenshot", BROWSER_OS_CAPTURESCREENSHOT)
+
+  BlockBrowserCaptureScreenshotFunction();
+
+ protected:
+  ~BlockBrowserCaptureScreenshotFunction() override;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+  
+ private:
+  void DrawHighlightsAndCapture();
+  void CaptureScreenshotNow();
+  void OnScreenshotCaptured(const SkBitmap& bitmap);
+  
+  // Store web contents and tab id for highlight operations
+  raw_ptr<content::WebContents> web_contents_ = nullptr;
+  int tab_id_ = -1;
+  gfx::Size target_size_;
+  bool show_highlights_ = false;
+  bool use_exact_dimensions_ = false;
+};
+
+class BlockBrowserGetSnapshotFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getSnapshot", BROWSER_OS_GETSNAPSHOT)
+
+  BlockBrowserGetSnapshotFunction() = default;
+
+ protected:
+  ~BlockBrowserGetSnapshotFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+
+ private:
+  void OnAccessibilityTreeReceived(ui::AXTreeUpdate& tree_update);
+};
+
+// Settings API functions
+class BlockBrowserGetPrefFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getPref", BROWSER_OS_GETPREF)
+
+  BlockBrowserGetPrefFunction() = default;
+
+ protected:
+  ~BlockBrowserGetPrefFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserSetPrefFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.setPref", BROWSER_OS_SETPREF)
+
+  BlockBrowserSetPrefFunction() = default;
+
+ protected:
+  ~BlockBrowserSetPrefFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserGetAllPrefsFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getAllPrefs", BROWSER_OS_GETALLPREFS)
+
+  BlockBrowserGetAllPrefsFunction() = default;
+
+ protected:
+  ~BlockBrowserGetAllPrefsFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserLogMetricFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.logMetric", BROWSER_OS_LOGMETRIC)
+
+  BlockBrowserLogMetricFunction() = default;
+
+ protected:
+  ~BlockBrowserLogMetricFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserGetVersionNumberFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.getVersionNumber", BROWSER_OS_GETVERSIONNUMBER)
+
+  BlockBrowserGetVersionNumberFunction() = default;
+
+ protected:
+  ~BlockBrowserGetVersionNumberFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserExecuteJavaScriptFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.executeJavaScript", BROWSER_OS_EXECUTEJAVASCRIPT)
+
+  BlockBrowserExecuteJavaScriptFunction() = default;
+
+ protected:
+  ~BlockBrowserExecuteJavaScriptFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+  
+ private:
+  void OnJavaScriptExecuted(base::Value result);
+};
+
+class BlockBrowserClickCoordinatesFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.clickCoordinates", BROWSER_OS_CLICKCOORDINATES)
+
+  BlockBrowserClickCoordinatesFunction() = default;
+
+ protected:
+  ~BlockBrowserClickCoordinatesFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BlockBrowserTypeAtCoordinatesFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("blockbrowser.typeAtCoordinates", BROWSER_OS_TYPEATCOORDINATES)
+
+  BlockBrowserTypeAtCoordinatesFunction() = default;
+
+ protected:
+  ~BlockBrowserTypeAtCoordinatesFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
\ No newline at end of file
