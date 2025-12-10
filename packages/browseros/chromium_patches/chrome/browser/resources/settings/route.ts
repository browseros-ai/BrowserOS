diff --git a/chrome/browser/resources/settings/route.ts b/chrome/browser/resources/settings/route.ts
index 1fd9c83cb74e7..8f1f644b92307 100644
--- a/chrome/browser/resources/settings/route.ts
+++ b/chrome/browser/resources/settings/route.ts
@@ -165,6 +165,8 @@ function createRoutes(): SettingsRoutes {
 
   // Root page.
   r.BASIC = new Route('/');
+  r.NXTSCAPE = new Route('/browseros-ai', 'BrowserOS AI Settings');
+  r.BROWSEROS_PREFS = new Route('/browseros-settings', 'BrowserOS Settings');
 
   r.ABOUT = r.BASIC.createSection(
       '/help', 'about', loadTimeData.getString('aboutPageTitle'));
