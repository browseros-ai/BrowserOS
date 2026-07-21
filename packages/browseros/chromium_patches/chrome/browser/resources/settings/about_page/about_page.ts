diff --git a/chrome/browser/resources/settings/about_page/about_page.ts b/chrome/browser/resources/settings/about_page/about_page.ts
index dd9d93e518b3665fa3745bf03cbc126804f55e65..63e0191dd755c2cbeb98b5a6b0b0afd82a39fc0a 100644
--- a/chrome/browser/resources/settings/about_page/about_page.ts
+++ b/chrome/browser/resources/settings/about_page/about_page.ts
@@ -222,7 +222,7 @@ export class SettingsAboutPageElement extends SettingsAboutPageElementBase
   }
 
   protected onHelpClick_() {
-    this.aboutBrowserProxy_.openHelpPage();
+    window.open('http://docs.browseros.com/');
   }
 
   protected onRelaunchClick_() {
