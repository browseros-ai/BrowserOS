diff --git a/tools/gritsettings/resource_ids.spec b/tools/gritsettings/resource_ids.spec
index ffd47250ce5a6..34bf8cf50a085 100644
--- a/tools/gritsettings/resource_ids.spec
+++ b/tools/gritsettings/resource_ids.spec
@@ -165,6 +165,10 @@
     "messages": [2540],
     "includes": [2600],
   },
+  "<(SHARED_INTERMEDIATE_DIR)/chrome/browser/browseros/onboarding/resources.grd": {
+    "META": {"sizes": {"includes": [20]}},
+    "includes": [2680],
+  },
   # END chrome/browser section.
 
   # START chrome/ WebUI resources section
