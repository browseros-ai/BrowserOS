diff --git a/chrome/browser/extensions/api/browser_os/browser_os_api.h b/chrome/browser/extensions/api/browser_os/browser_os_api.h
new file mode 100644
index 0000000000000..53ece24723da0
--- /dev/null
+++ b/chrome/browser/extensions/api/browser_os/browser_os_api.h
@@ -0,0 +1,371 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
+#define CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
+
+#include <cstdint>
+
+#include "base/memory/weak_ptr.h"
+#include "base/values.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_api_utils.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_content_processor.h"
+#include "chrome/browser/extensions/api/browser_os/browser_os_snapshot_processor.h"
+#include "components/viz/common/frame_sinks/copy_output_result.h"
+#include "content/public/browser/render_widget_host_view.h"
+#include "extensions/browser/extension_function.h"
+#include "third_party/skia/include/core/SkBitmap.h"
+#include "ui/shell_dialogs/select_file_dialog.h"
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
+class BrowserOSGetAccessibilityTreeFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getAccessibilityTree",
+                             BROWSER_OS_GETACCESSIBILITYTREE)
+
+  BrowserOSGetAccessibilityTreeFunction() = default;
+
+ protected:
+  ~BrowserOSGetAccessibilityTreeFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+
+ private:
+  void OnAccessibilityTreeReceived(ui::AXTreeUpdate& tree_update);
+};
+
+class BrowserOSGetInteractiveSnapshotFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getInteractiveSnapshot",
+                             BROWSER_OS_GETINTERACTIVESNAPSHOT)
+
+  BrowserOSGetInteractiveSnapshotFunction();
+
+ protected:
+  ~BrowserOSGetInteractiveSnapshotFunction() override;
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
+  base::WeakPtr<content::WebContents> web_contents_;
+};
+
+class BrowserOSClickFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.click", BROWSER_OS_CLICK)
+
+  BrowserOSClickFunction() = default;
+
+ protected:
+  ~BrowserOSClickFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSInputTextFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.inputText", BROWSER_OS_INPUTTEXT)
+
+  BrowserOSInputTextFunction() = default;
+
+ protected:
+  ~BrowserOSInputTextFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSClearFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.clear", BROWSER_OS_CLEAR)
+
+  BrowserOSClearFunction() = default;
+
+ protected:
+  ~BrowserOSClearFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSGetPageLoadStatusFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getPageLoadStatus", 
+                             BROWSER_OS_GETPAGELOADSTATUS)
+
+  BrowserOSGetPageLoadStatusFunction() = default;
+
+ protected:
+  ~BrowserOSGetPageLoadStatusFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSScrollUpFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.scrollUp", BROWSER_OS_SCROLLUP)
+
+  BrowserOSScrollUpFunction() = default;
+
+ protected:
+  ~BrowserOSScrollUpFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSScrollDownFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.scrollDown", BROWSER_OS_SCROLLDOWN)
+
+  BrowserOSScrollDownFunction() = default;
+
+ protected:
+  ~BrowserOSScrollDownFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSScrollToNodeFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.scrollToNode", BROWSER_OS_SCROLLTONODE)
+
+  BrowserOSScrollToNodeFunction() = default;
+
+ protected:
+  ~BrowserOSScrollToNodeFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSSendKeysFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.sendKeys", BROWSER_OS_SENDKEYS)
+
+  BrowserOSSendKeysFunction() = default;
+
+ protected:
+  ~BrowserOSSendKeysFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSCaptureScreenshotFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.captureScreenshot", BROWSER_OS_CAPTURESCREENSHOT)
+
+  BrowserOSCaptureScreenshotFunction();
+
+ protected:
+  ~BrowserOSCaptureScreenshotFunction() override;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+  
+ private:
+  void DrawHighlightsAndCapture();
+  void CaptureScreenshotNow();
+  void OnScreenshotCaptured(const content::CopyFromSurfaceResult& result);
+  
+  // Store web contents and tab id for highlight operations
+  base::WeakPtr<content::WebContents> web_contents_;
+  int tab_id_ = -1;
+  gfx::Size target_size_;
+  bool show_highlights_ = false;
+  bool use_exact_dimensions_ = false;
+};
+
+class BrowserOSGetSnapshotFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getSnapshot", BROWSER_OS_GETSNAPSHOT)
+
+  BrowserOSGetSnapshotFunction() = default;
+
+ protected:
+  ~BrowserOSGetSnapshotFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+
+ private:
+  void OnAccessibilityTreeReceived(ui::AXTreeUpdate& tree_update);
+};
+
+// Settings API functions
+class BrowserOSGetPrefFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getPref", BROWSER_OS_GETPREF)
+
+  BrowserOSGetPrefFunction() = default;
+
+ protected:
+  ~BrowserOSGetPrefFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSSetPrefFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.setPref", BROWSER_OS_SETPREF)
+
+  BrowserOSSetPrefFunction() = default;
+
+ protected:
+  ~BrowserOSSetPrefFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSGetAllPrefsFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getAllPrefs", BROWSER_OS_GETALLPREFS)
+
+  BrowserOSGetAllPrefsFunction() = default;
+
+ protected:
+  ~BrowserOSGetAllPrefsFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSLogMetricFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.logMetric", BROWSER_OS_LOGMETRIC)
+
+  BrowserOSLogMetricFunction() = default;
+
+ protected:
+  ~BrowserOSLogMetricFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSGetVersionNumberFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getVersionNumber", BROWSER_OS_GETVERSIONNUMBER)
+
+  BrowserOSGetVersionNumberFunction() = default;
+
+ protected:
+  ~BrowserOSGetVersionNumberFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSGetBrowserosVersionNumberFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.getBrowserosVersionNumber", BROWSER_OS_GETBROWSEROSVERSIONNUMBER)
+
+  BrowserOSGetBrowserosVersionNumberFunction() = default;
+
+ protected:
+  ~BrowserOSGetBrowserosVersionNumberFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSExecuteJavaScriptFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.executeJavaScript", BROWSER_OS_EXECUTEJAVASCRIPT)
+
+  BrowserOSExecuteJavaScriptFunction() = default;
+
+ protected:
+  ~BrowserOSExecuteJavaScriptFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+  
+ private:
+  void OnJavaScriptExecuted(base::Value result);
+};
+
+class BrowserOSClickCoordinatesFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.clickCoordinates", BROWSER_OS_CLICKCOORDINATES)
+
+  BrowserOSClickCoordinatesFunction() = default;
+
+ protected:
+  ~BrowserOSClickCoordinatesFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSTypeAtCoordinatesFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.typeAtCoordinates", BROWSER_OS_TYPEATCOORDINATES)
+
+  BrowserOSTypeAtCoordinatesFunction() = default;
+
+ protected:
+  ~BrowserOSTypeAtCoordinatesFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
+class BrowserOSChoosePathFunction : public ExtensionFunction,
+                                    public ui::SelectFileDialog::Listener {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.choosePath", BROWSER_OS_CHOOSEPATH)
+
+  BrowserOSChoosePathFunction();
+  BrowserOSChoosePathFunction(const BrowserOSChoosePathFunction&) = delete;
+  BrowserOSChoosePathFunction& operator=(const BrowserOSChoosePathFunction&) =
+      delete;
+
+  // ui::SelectFileDialog::Listener:
+  void FileSelected(const ui::SelectedFileInfo& file, int index) override;
+  void FileSelectionCanceled() override;
+
+ protected:
+  ~BrowserOSChoosePathFunction() override;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+
+ private:
+  scoped_refptr<ui::SelectFileDialog> select_file_dialog_;
+};
+
+}  // namespace api
+}  // namespace extensions
+
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
