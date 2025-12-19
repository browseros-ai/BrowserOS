diff --git a/chrome/browser/mac/chrome_browser_main_extra_parts_mac.mm b/chrome/browser/mac/chrome_browser_main_extra_parts_mac.mm
index 6bb5ccb823895..b6bed8c40d57d 100644
--- a/chrome/browser/mac/chrome_browser_main_extra_parts_mac.mm
+++ b/chrome/browser/mac/chrome_browser_main_extra_parts_mac.mm
@@ -4,11 +4,25 @@
 
 #include "chrome/browser/mac/chrome_browser_main_extra_parts_mac.h"
 
+#include "chrome/browser/buildflags.h"
 #include "ui/display/screen.h"
 
+#if BUILDFLAG(ENABLE_SPARKLE)
+#include "chrome/browser/mac/sparkle_glue.h"
+#endif
+
 ChromeBrowserMainExtraPartsMac::ChromeBrowserMainExtraPartsMac() = default;
 ChromeBrowserMainExtraPartsMac::~ChromeBrowserMainExtraPartsMac() = default;
 
 void ChromeBrowserMainExtraPartsMac::PreEarlyInitialization() {
   screen_ = std::make_unique<display::ScopedNativeScreen>();
 }
+
+void ChromeBrowserMainExtraPartsMac::PreCreateMainMessageLoop() {
+#if BUILDFLAG(ENABLE_SPARKLE)
+  // Initialize Sparkle. This triggers the singleton creation which handles
+  // all setup internally, including checking if updates are disabled or
+  // if running from a read-only filesystem.
+  sparkle_glue::SparkleEnabled();
+#endif
+}
