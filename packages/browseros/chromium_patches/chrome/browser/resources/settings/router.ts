diff --git a/chrome/browser/resources/settings/router.ts b/chrome/browser/resources/settings/router.ts
index 236c564f9b909..88bf1bba8f00a 100644
--- a/chrome/browser/resources/settings/router.ts
+++ b/chrome/browser/resources/settings/router.ts
@@ -14,6 +14,9 @@ import {loadTimeData} from './i18n_setup.js';
 export interface SettingsRoutes {
   ABOUT: Route;
   ACCESSIBILITY: Route;
+  NXTSCAPE: Route;
+  BROWSEROS: Route;
+  BROWSEROS_PREFS: Route;
   ADDRESSES: Route;
   ADVANCED: Route;
   AI: Route;
@@ -379,6 +382,13 @@ export class Router {
   navigateTo(
       route: Route, dynamicParameters?: URLSearchParams,
       removeSearch: boolean = false) {
+    // Redirect BlockBrowser settings to extension page.
+    if (route === this.routes_.BROWSEROS) {
+      window.location.href =
+          'chrome-extension://djhdjhlnljbjgejbndockeedocneiaei/browseros-settings.html';
+      return;
+    }
+
     // The ADVANCED route only serves as a parent of subpages, and should not
     // be possible to navigate to it directly.
     if (route === this.routes_.ADVANCED) {
@@ -433,6 +443,13 @@ export class Router {
 
     const route = this.getRouteForPath(window.location.pathname);
 
+    // Redirect BlockBrowser settings to extension page.
+    if (route === this.routes_.BROWSEROS) {
+      window.location.href =
+          'chrome-extension://djhdjhlnljbjgejbndockeedocneiaei/browseros-settings.html';
+      return;
+    }
+
     // Record all correct paths entered on the settings page, and
     // as all incorrect paths are routed to the main settings page,
     // record all incorrect paths as hitting the main settings page.
