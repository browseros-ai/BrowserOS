diff --git a/chrome/browser/profiles/profile_window.cc b/chrome/browser/profiles/profile_window.cc
index c13b772bc2eae7e744e53374733b7d40595dd78c..6881556779f7a8e4f656e863229d338a3f8ff6c3 100644
--- a/chrome/browser/profiles/profile_window.cc
+++ b/chrome/browser/profiles/profile_window.cc
@@ -6,6 +6,9 @@
 
 #include <stddef.h>
 
+#include <utility>
+#include <vector>
+
 #include "base/command_line.h"
 #include "base/debug/stack_trace.h"
 #include "base/files/file_path.h"
@@ -87,7 +90,8 @@ void FindOrCreateNewWindowForProfile(
     chrome::startup::IsProcessStartup process_startup,
     chrome::startup::IsFirstRun is_first_run,
     bool always_create,
-    bool open_command_line_urls) {
+    bool open_command_line_urls,
+    std::vector<GURL> first_run_tabs) {
   DCHECK(profile);
   TRACE_EVENT1("browser", "FindOrCreateNewWindowForProfile", "profile_path",
                profile->GetPath());
@@ -104,6 +108,7 @@ void FindOrCreateNewWindowForProfile(
   base::RecordAction(UserMetricsAction("NewWindow"));
   base::CommandLine command_line(base::CommandLine::NO_PROGRAM);
   StartupBrowserCreator browser_creator;
+  browser_creator.AddFirstRunTabs(first_run_tabs);
 
 #if !BUILDFLAG(IS_CHROMEOS)
   if (open_command_line_urls) {
@@ -130,6 +135,18 @@ void OpenBrowserWindowForProfile(base::OnceCallback<void(Browser*)> callback,
                                  bool is_new_profile,
                                  bool open_command_line_urls,
                                  Profile* profile) {
+  OpenBrowserWindowForProfileWithFirstRunTabs(
+      std::move(callback), always_create, is_new_profile,
+      open_command_line_urls, profile, std::vector<GURL>());
+}
+
+void OpenBrowserWindowForProfileWithFirstRunTabs(
+    base::OnceCallback<void(Browser*)> callback,
+    bool always_create,
+    bool is_new_profile,
+    bool open_command_line_urls,
+    Profile* profile,
+    std::vector<GURL> first_run_tabs) {
   DCHECK_CURRENTLY_ON(BrowserThread::UI);
   TRACE_EVENT1("browser", "OpenBrowserWindowForProfile", "profile_path",
                profile->GetPath().AsUTF8Unsafe());
@@ -199,7 +216,8 @@ void OpenBrowserWindowForProfile(base::OnceCallback<void(Browser*)> callback,
   // Passing true for |always_create| means we won't duplicate the code that
   // tries to find a browser.
   profiles::FindOrCreateNewWindowForProfile(
-      profile, process_startup, is_first_run, true, open_command_line_urls);
+      profile, process_startup, is_first_run, true, open_command_line_urls,
+      std::move(first_run_tabs));
 }
 
 #if !BUILDFLAG(IS_ANDROID)
