diff --git a/chrome/browser/resources/settings/about_page/about_page.ts b/chrome/browser/resources/settings/about_page/about_page.ts
index aa3f435d831b0..09bcb0efa074a 100644
--- a/chrome/browser/resources/settings/about_page/about_page.ts
+++ b/chrome/browser/resources/settings/about_page/about_page.ts
@@ -215,7 +215,7 @@ export class SettingsAboutPageElement extends SettingsAboutPageElementBase
   }
 
   private onHelpClick_() {
-    this.aboutBrowserProxy_.openHelpPage();
+    window.open('http://docs.browseros.com/');
   }
 
   private onRelaunchClick_() {
