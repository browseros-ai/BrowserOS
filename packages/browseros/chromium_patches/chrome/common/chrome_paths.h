diff --git a/chrome/common/chrome_paths.h b/chrome/common/chrome_paths.h
index 9b3e9be1f14c1..401ef81129ecf 100644
--- a/chrome/common/chrome_paths.h
+++ b/chrome/common/chrome_paths.h
@@ -131,6 +131,9 @@ enum {
   DIR_OPTIMIZATION_GUIDE_PREDICTION_MODELS,  // Directory where verified models
                                              // downloaded by the Optimization
                                              // Guide are stored.
+  DIR_BROWSEROS_BUNDLED_EXTENSIONS,  // Directory containing bundled BrowserOS
+                                     // extension CRX files for immediate
+                                     // installation on first run.
   PATH_END
 };
 
