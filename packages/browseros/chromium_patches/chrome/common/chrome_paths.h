diff --git a/chrome/common/chrome_paths.h b/chrome/common/chrome_paths.h
index e36249713bfd5c254a322c3c55fd31d129937fab..c921498ec328004f82b0661a2e84826ef8d49843 100644
--- a/chrome/common/chrome_paths.h
+++ b/chrome/common/chrome_paths.h
@@ -126,6 +126,9 @@ enum {
   DIR_OPTIMIZATION_GUIDE_PREDICTION_MODELS,  // Directory where verified models
                                              // downloaded by the Optimization
                                              // Guide are stored.
+  DIR_BROWSEROS_BUNDLED_EXTENSIONS,  // Directory containing bundled BrowserOS
+                                     // extension CRX files for immediate
+                                     // installation on first run.
   PATH_END
 };
 
