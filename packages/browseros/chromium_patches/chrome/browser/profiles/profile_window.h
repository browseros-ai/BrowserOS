diff --git a/chrome/browser/profiles/profile_window.h b/chrome/browser/profiles/profile_window.h
index 7f54ef0e8b029..be2f4098dec37 100644
--- a/chrome/browser/profiles/profile_window.h
+++ b/chrome/browser/profiles/profile_window.h
@@ -5,6 +5,8 @@
 #ifndef CHROME_BROWSER_PROFILES_PROFILE_WINDOW_H_
 #define CHROME_BROWSER_PROFILES_PROFILE_WINDOW_H_
 
+#include <vector>
+
 #include "base/functional/callback.h"
 #include "base/memory/raw_ptr.h"
 #include "base/memory/weak_ptr.h"
@@ -12,6 +14,7 @@
 #include "build/build_config.h"
 #include "chrome/browser/profiles/profile_observer.h"
 #include "chrome/browser/ui/browser_window/public/browser_collection_observer.h"
+#include "url/gurl.h"
 
 #if BUILDFLAG(IS_ANDROID)
 #error "Not used on Android"
@@ -44,7 +47,8 @@ void FindOrCreateNewWindowForProfile(
     chrome::startup::IsProcessStartup process_startup,
     chrome::startup::IsFirstRun is_first_run,
     bool always_create,
-    bool open_command_line_urls = false);
+    bool open_command_line_urls = false,
+    std::vector<GURL> first_run_tabs = {});
 
 // Opens a Browser for |profile|.
 // If |always_create| is true a window is created even if one already exists.
@@ -59,6 +63,14 @@ void OpenBrowserWindowForProfile(base::OnceCallback<void(Browser*)> callback,
                                  bool open_command_line_urls,
                                  Profile* profile);
 
+void OpenBrowserWindowForProfileWithFirstRunTabs(
+    base::OnceCallback<void(Browser*)> callback,
+    bool always_create,
+    bool is_new_profile,
+    bool open_command_line_urls,
+    Profile* profile,
+    std::vector<GURL> first_run_tabs);
+
 // Loads the specified profile given by |path| asynchronously. Once profile is
 // loaded and initialized it runs |callback| if it isn't null.
 void LoadProfileAsync(const base::FilePath& path,
