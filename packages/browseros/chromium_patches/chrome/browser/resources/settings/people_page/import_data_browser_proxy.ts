diff --git a/chrome/browser/resources/settings/people_page/import_data_browser_proxy.ts b/chrome/browser/resources/settings/people_page/import_data_browser_proxy.ts
index c4e401c551fc5..d2ca28cda54fa 100644
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
