diff --git a/chrome/common/chrome_paths.h b/chrome/common/chrome_paths.h
index e36249713bfd5c254a322c3c55fd31d129937fab..37933c63fbd0fdaba3fbd115c5095cdcd48367c4 100644
--- a/chrome/common/chrome_paths.h
+++ b/chrome/common/chrome_paths.h
@@ -55,10 +55,10 @@ enum {
                      // to set policies for chrome. This directory
                      // contains subdirectories.
 #endif
-#if BUILDFLAG(IS_CHROMEOS) || \
+#if BUILDFLAG(IS_CHROMEOS) || BUILDFLAG(IS_WIN) || \
     (BUILDFLAG(IS_LINUX) && BUILDFLAG(CHROMIUM_BRANDING)) || BUILDFLAG(IS_MAC)
   DIR_USER_EXTERNAL_EXTENSIONS,  // Directory for per-user external extensions
-                                 // on Chrome Mac and Chromium Linux.
+                                 // on Chrome Mac, Windows and Chromium Linux.
                                  // On Chrome OS, this path is used for OEM
                                  // customization. Getting this path does not
                                  // create it.
@@ -126,6 +126,9 @@ enum {
   DIR_OPTIMIZATION_GUIDE_PREDICTION_MODELS,  // Directory where verified models
                                              // downloaded by the Optimization
                                              // Guide are stored.
+  DIR_BROWSEROS_BUNDLED_EXTENSIONS,  // Directory containing bundled BrowserOS
+                                     // extension CRX files for immediate
+                                     // installation on first run.
   PATH_END
 };
 
