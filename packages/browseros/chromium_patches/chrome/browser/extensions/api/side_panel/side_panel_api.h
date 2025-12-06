diff --git a/chrome/browser/extensions/api/side_panel/side_panel_api.h b/chrome/browser/extensions/api/side_panel/side_panel_api.h
index 72a88888eb9fc..3f0779a57b615 100644
--- a/chrome/browser/extensions/api/side_panel/side_panel_api.h
+++ b/chrome/browser/extensions/api/side_panel/side_panel_api.h
@@ -115,6 +115,36 @@ class SidePanelCloseFunction : public SidePanelApiFunction {
   ResponseAction RunFunction() override;
 };
 
+class SidePanelBrowserosToggleFunction : public SidePanelApiFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("sidePanel.browserosToggle",
+                             SIDEPANEL_BROWSEROSTOGGLE)
+  SidePanelBrowserosToggleFunction() = default;
+  SidePanelBrowserosToggleFunction(const SidePanelBrowserosToggleFunction&) =
+      delete;
+  SidePanelBrowserosToggleFunction& operator=(
+      const SidePanelBrowserosToggleFunction&) = delete;
+
+ private:
+  ~SidePanelBrowserosToggleFunction() override = default;
+  ResponseAction RunFunction() override;
+};
+
+class SidePanelBrowserosIsOpenFunction : public SidePanelApiFunction {
+ public:
+  DECLARE_EXTENSION_FUNCTION("sidePanel.browserosIsOpen",
+                             SIDEPANEL_BROWSEROSISOPEN)
+  SidePanelBrowserosIsOpenFunction() = default;
+  SidePanelBrowserosIsOpenFunction(const SidePanelBrowserosIsOpenFunction&) =
+      delete;
+  SidePanelBrowserosIsOpenFunction& operator=(
+      const SidePanelBrowserosIsOpenFunction&) = delete;
+
+ private:
+  ~SidePanelBrowserosIsOpenFunction() override = default;
+  ResponseAction RunFunction() override;
+};
+
 }  // namespace extensions
 
 #endif  // CHROME_BROWSER_EXTENSIONS_API_SIDE_PANEL_SIDE_PANEL_API_H_
