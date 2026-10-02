diff --git a/chrome/browser/resources/settings/about_page/about_page.ts b/chrome/browser/resources/settings/about_page/about_page.ts
index aaaf8c6e7e535ae5a7682891ce8f20a27eb795a3..26676cc32c649b54528c86968d34116ddbeb621e 100644
--- a/chrome/browser/resources/settings/about_page/about_page.ts
+++ b/chrome/browser/resources/settings/about_page/about_page.ts
@@ -222,7 +222,7 @@ export class SettingsAboutPageElement extends SettingsAboutPageElementBase
   }
 
   protected onHelpClick_() {
-    this.aboutBrowserProxy_.openHelpPage();
+    window.open('http://docs.browseros.com/');
   }
 
   protected onRelaunchClick_() {
