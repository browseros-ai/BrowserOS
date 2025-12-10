diff --git a/chrome/browser/resources/settings/router.ts b/chrome/browser/resources/settings/router.ts
index 09b29e00e7c1a..637784bb98618 100644
--- a/chrome/browser/resources/settings/router.ts
+++ b/chrome/browser/resources/settings/router.ts
@@ -14,6 +14,8 @@ import {loadTimeData} from './i18n_setup.js';
 export interface SettingsRoutes {
   ABOUT: Route;
   ACCESSIBILITY: Route;
+  NXTSCAPE: Route;
+  BROWSEROS_PREFS: Route;
   ADDRESSES: Route;
   ADVANCED: Route;
   AI: Route;
