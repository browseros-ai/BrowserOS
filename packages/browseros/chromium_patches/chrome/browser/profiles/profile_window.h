diff --git a/chrome/browser/profiles/profile_window.h b/chrome/browser/profiles/profile_window.h
index c15abac6889a1f7b642bca80ce4aa97a42406738..cb4b66df594f048bdfc4a1dcca0ca667a34b8cc1 100644
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
@@ -43,7 +46,8 @@ void FindOrCreateNewWindowForProfile(
     chrome::startup::IsProcessStartup process_startup,
     chrome::startup::IsFirstRun is_first_run,
     bool always_create,
-    bool open_command_line_urls = false);
+    bool open_command_line_urls = false,
+    std::vector<GURL> first_run_tabs = {});
 
 // Opens a Browser for |profile|.
 // If |always_create| is true a window is created even if one already exists.
@@ -59,6 +63,16 @@ void OpenBrowserWindowForProfile(
     bool open_command_line_urls,
     Profile* profile);
 
+// Like OpenBrowserWindowForProfile, but supplies tabs to startup before the
+// window-created callback can clear the onboarding picker and its keep-alive.
+void OpenBrowserWindowForProfileWithFirstRunTabs(
+    base::OnceCallback<void(BrowserWindowInterface*)> callback,
+    bool always_create,
+    bool is_new_profile,
+    bool open_command_line_urls,
+    Profile* profile,
+    std::vector<GURL> first_run_tabs);
+
 // Loads the specified profile given by |path| asynchronously. Once profile is
 // loaded and initialized it runs |callback| if it isn't null.
 void LoadProfileAsync(const base::FilePath& path,
