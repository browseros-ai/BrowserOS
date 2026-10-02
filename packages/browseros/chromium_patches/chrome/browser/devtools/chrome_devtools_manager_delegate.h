diff --git a/chrome/browser/devtools/chrome_devtools_manager_delegate.h b/chrome/browser/devtools/chrome_devtools_manager_delegate.h
index 5a1fb503d2ce171289b6ac406fdaff303f76a28c..82b7803465f3db5b8226c4b91dcaa651f59543bf 100644
--- a/chrome/browser/devtools/chrome_devtools_manager_delegate.h
+++ b/chrome/browser/devtools/chrome_devtools_manager_delegate.h
@@ -83,6 +83,9 @@ class ChromeDevToolsManagerDelegate : public content::DevToolsManagerDelegate,
       content::DevToolsAgentHost* agent_host) override;
   std::optional<bool> ShouldReportAsTabTarget(
       content::WebContents* web_contents) override;
+  bool GetTargetTabId(content::WebContents* web_contents,
+                      int* tab_id,
+                      int* window_id) override;
 
   content::BrowserContext* CreateBrowserContext() override;
   void DisposeBrowserContext(content::BrowserContext*,
