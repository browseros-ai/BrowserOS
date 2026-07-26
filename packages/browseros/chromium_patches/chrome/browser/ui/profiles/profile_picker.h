diff --git a/chrome/browser/ui/profiles/profile_picker.h b/chrome/browser/ui/profiles/profile_picker.h
index 4507686f20b87..661b06acb9a96 100644
--- a/chrome/browser/ui/profiles/profile_picker.h
+++ b/chrome/browser/ui/profiles/profile_picker.h
@@ -7,6 +7,7 @@
 
 #include <optional>
 #include <variant>
+#include <vector>
 
 #include "base/files/file_path.h"
 #include "base/functional/callback.h"
@@ -323,6 +324,10 @@ class ProfilePicker {
   // Opens the command line urls in the next profile that is opened.
   static void SetOpenCommandLineUrlsInNextProfileOpened(bool value);
   static bool GetOpenCommandLineUrlsInNextProfileOpened();
+
+  // Opens first-run tabs in the next profile that is opened.
+  static void SetFirstRunTabsInNextProfileOpened(std::vector<GURL> urls);
+  static std::vector<GURL> TakeFirstRunTabsInNextProfileOpened();
 };
 
 #endif  // CHROME_BROWSER_UI_PROFILES_PROFILE_PICKER_H_
