diff --git a/chrome/browser/ui/profiles/profile_picker.h b/chrome/browser/ui/profiles/profile_picker.h
index f32c4a46c50e963567ed898e3d57935139054507..b97adc62c1428728dbe63e6f6f9a2875e677b3d0 100644
--- a/chrome/browser/ui/profiles/profile_picker.h
+++ b/chrome/browser/ui/profiles/profile_picker.h
@@ -7,6 +7,7 @@
 
 #include <optional>
 #include <variant>
+#include <vector>
 
 #include "base/files/file_path.h"
 #include "base/functional/callback.h"
@@ -328,6 +329,10 @@ class ProfilePicker {
   // Opens the command line urls in the next profile that is opened.
   static void SetOpenCommandLineUrlsInNextProfileOpened(bool value);
   static bool GetOpenCommandLineUrlsInNextProfileOpened();
+
+  // Opens first-run tabs in the next profile that is opened.
+  static void SetFirstRunTabsInNextProfileOpened(std::vector<GURL> urls);
+  static std::vector<GURL> TakeFirstRunTabsInNextProfileOpened();
 };
 
 #endif  // CHROME_BROWSER_UI_PROFILES_PROFILE_PICKER_H_
