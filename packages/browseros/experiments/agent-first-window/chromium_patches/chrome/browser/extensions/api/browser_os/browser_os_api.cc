diff --git a/chrome/browser/extensions/api/browser_os/browser_os_api.cc b/chrome/browser/extensions/api/browser_os/browser_os_api.cc
index 0022c6ea0fe1b7d6247940f3861d447c1983ba8d..9f0fce23108a0adea54c6e603ffc8e055ef6640b 100644
--- a/chrome/browser/extensions/api/browser_os/browser_os_api.cc
+++ b/chrome/browser/extensions/api/browser_os/browser_os_api.cc
@@ -32,6 +32,7 @@
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/browser_finder.h"
 #include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/common/extensions/api/browser_os.h"
 #include "content/browser/renderer_host/render_widget_host_impl.h"
 #include "content/public/browser/render_frame_host.h"
@@ -1230,6 +1231,23 @@ void BrowserOSExecuteJavaScriptFunction::OnJavaScriptExecuted(base::Value result
       browser_os::ExecuteJavaScript::Results::Create(result)));
 }
 
+ExtensionFunction::ResponseAction BrowserOSToggleAgentSplitFunction::Run() {
+  content::WebContents* sender = GetSenderWebContents();
+  if (!sender) {
+    return RespondNow(Error("Could not find the calling extension page."));
+  }
+
+  BrowserView* browser_view = BrowserView::GetBrowserViewForNativeWindow(
+      sender->GetTopLevelNativeWindow());
+  if (!browser_view) {
+    return RespondNow(
+        Error("Could not find the browser window that owns the caller."));
+  }
+
+  browser_view->ToggleAgentSplit();
+  return RespondNow(NoArguments());
+}
+
 // Implementation of BrowserOSClickCoordinatesFunction
 ExtensionFunction::ResponseAction BrowserOSClickCoordinatesFunction::Run() {
   std::optional<browser_os::ClickCoordinates::Params> params =
