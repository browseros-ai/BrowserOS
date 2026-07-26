diff --git a/chrome/browser/resources/settings/people_page/import_data_browser_proxy.ts b/chrome/browser/resources/settings/people_page/import_data_browser_proxy.ts
index ca628e702953f..263fad958f5d1 100644
--- a/chrome/browser/resources/settings/people_page/import_data_browser_proxy.ts
+++ b/chrome/browser/resources/settings/people_page/import_data_browser_proxy.ts
@@ -19,6 +19,8 @@ export interface BrowserProfile {
   passwords: boolean;
   search: boolean;
   autofillFormData: boolean;
+  extensions: boolean;
+  cookies: boolean;
 }
 
 /**
