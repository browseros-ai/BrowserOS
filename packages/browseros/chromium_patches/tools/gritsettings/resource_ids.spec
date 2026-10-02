diff --git a/tools/gritsettings/resource_ids.spec b/tools/gritsettings/resource_ids.spec
index c90c7c5e5521e0355bc2df9c3435b500a6613118..8b58fe420a4c1327cc603b43a10e4ed246ea573f 100644
--- a/tools/gritsettings/resource_ids.spec
+++ b/tools/gritsettings/resource_ids.spec
@@ -192,6 +192,10 @@
   "chrome/browser/indigo/resources/browser_resources.grd": {
     "includes": [2640],
   },
+  "<(SHARED_INTERMEDIATE_DIR)/chrome/browser/browseros/onboarding/resources.grd": {
+    "META": {"sizes": {"includes": [20]}},
+    "includes": [2680],
+  },
   # END chrome/browser section.
 
   # START chrome/ WebUI resources section
