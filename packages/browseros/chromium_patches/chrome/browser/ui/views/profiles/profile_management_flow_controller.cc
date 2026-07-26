diff --git a/chrome/browser/ui/views/profiles/profile_management_flow_controller.cc b/chrome/browser/ui/views/profiles/profile_management_flow_controller.cc
index b97220b8d91e6..6a1d1f2f237db 100644
--- a/chrome/browser/ui/views/profiles/profile_management_flow_controller.cc
+++ b/chrome/browser/ui/views/profiles/profile_management_flow_controller.cc
@@ -4,6 +4,9 @@
 
 #include "chrome/browser/ui/views/profiles/profile_management_flow_controller.h"
 
+#include <utility>
+#include <vector>
+
 #include "base/check_is_test.h"
 #include "base/functional/bind.h"
 #include "base/functional/callback.h"
@@ -176,18 +179,21 @@ void ProfileManagementFlowController::FinishFlowAndRunInBrowser(
             .Then(std::move(post_host_cleared_callback.value()));
   }
 
-  bool open_command_line_urls = ProfilePicker::GetOpenCommandLineUrlsInNextProfileOpened();
+  bool open_command_line_urls =
+      ProfilePicker::GetOpenCommandLineUrlsInNextProfileOpened();
   ProfilePicker::SetOpenCommandLineUrlsInNextProfileOpened(false);
+  std::vector<GURL> first_run_tabs =
+      ProfilePicker::TakeFirstRunTabsInNextProfileOpened();
 
   // Start by opening the browser window, to ensure that we have another
   // KeepAlive for `profile` by the time we clear the flow and its host.
   // TODO(crbug.com/40242414): Make sure we do something or log an error if
   // opening a browser window was not possible.
-  profiles::OpenBrowserWindowForProfile(
+  profiles::OpenBrowserWindowForProfileWithFirstRunTabs(
       std::move(post_browser_open_callback),
       /*always_create=*/false,   // Don't create a window if one already exists.
       /*is_new_profile=*/false,  // Don't create a first run window.
-      open_command_line_urls, profile);
+      open_command_line_urls, profile, std::move(first_run_tabs));
 }
 
 base::OnceClosure
