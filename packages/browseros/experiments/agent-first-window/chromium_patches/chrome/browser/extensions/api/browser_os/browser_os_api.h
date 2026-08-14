diff --git a/chrome/browser/extensions/api/browser_os/browser_os_api.h b/chrome/browser/extensions/api/browser_os/browser_os_api.h
index e4b1c5f8213427183f0db933337b140a342db365..a7b5738d08edb7b2269b84225e01fa9270e39cf5 100644
--- a/chrome/browser/extensions/api/browser_os/browser_os_api.h
+++ b/chrome/browser/extensions/api/browser_os/browser_os_api.h
@@ -299,6 +299,20 @@ class BrowserOSExecuteJavaScriptFunction : public ExtensionFunction {
   void OnJavaScriptExecuted(base::Value result);
 };
 
+class BrowserOSToggleAgentSplitFunction : public ExtensionFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("browserOS.toggleAgentSplit",
+                             BROWSER_OS_TOGGLEAGENTSPLIT)
+
+  BrowserOSToggleAgentSplitFunction() = default;
+
+ protected:
+  ~BrowserOSToggleAgentSplitFunction() override = default;
+
+  // ExtensionFunction:
+  ResponseAction Run() override;
+};
+
 class BrowserOSClickCoordinatesFunction : public ExtensionFunction {
  public:
   DECLARE_EXTENSION_FUNCTION("browserOS.clickCoordinates", BROWSER_OS_CLICKCOORDINATES)
@@ -328,4 +342,4 @@ class BrowserOSTypeAtCoordinatesFunction : public ExtensionFunction {
 }  // namespace api
 }  // namespace extensions
 
-#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
\ No newline at end of file
+#endif  // CHROME_BROWSER_EXTENSIONS_API_BROWSER_OS_BROWSER_OS_API_H_
