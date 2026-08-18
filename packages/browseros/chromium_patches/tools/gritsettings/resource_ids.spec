diff --git a/tools/gritsettings/resource_ids.spec b/tools/gritsettings/resource_ids.spec
index e1f3030f0c0cc1b870cbe75395539d37ca1451dd..6d6b14c4fcc6260dc94898e09f4f2d3aca5055c0 100644
--- a/tools/gritsettings/resource_ids.spec
+++ b/tools/gritsettings/resource_ids.spec
@@ -189,6 +189,10 @@
   "chrome/browser/indigo/resources/browser_resources.grd": {
     "includes": [2640],
   },
+  "<(SHARED_INTERMEDIATE_DIR)/chrome/browser/browseros/onboarding/resources.grd": {
+    "META": {"sizes": {"includes": [20]}},
+    "includes": [2680],
+  },
   # END chrome/browser section.
 
   # START chrome/ WebUI resources section
