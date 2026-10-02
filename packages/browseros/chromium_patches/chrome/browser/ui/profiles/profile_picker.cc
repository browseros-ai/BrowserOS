diff --git a/chrome/browser/ui/profiles/profile_picker.cc b/chrome/browser/ui/profiles/profile_picker.cc
index cc2ae52597a8d1881a14d255d9b44296c8a0e4b5..98122b54188b7acbc5d3353d7b982fd13923a70a 100644
--- a/chrome/browser/ui/profiles/profile_picker.cc
+++ b/chrome/browser/ui/profiles/profile_picker.cc
@@ -5,6 +5,8 @@
 #include "chrome/browser/ui/profiles/profile_picker.h"
 
 #include <string>
+#include <utility>
+#include <vector>
 
 #include "base/check_deref.h"
 #include "base/check_is_test.h"
@@ -13,6 +15,7 @@
 #include "base/feature_list.h"
 #include "base/logging.h"
 #include "base/metrics/histogram_functions.h"
+#include "base/no_destructor.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/profiles/profile_manager.h"
@@ -33,6 +36,11 @@ namespace {
 
 bool g_open_command_line_urls_in_next_profile_opened = false;
 
+std::vector<GURL>& FirstRunTabsInNextProfileOpened() {
+  static base::NoDestructor<std::vector<GURL>> first_run_tabs;
+  return *first_run_tabs;
+}
+
 ProfilePicker::AvailabilityOnStartup GetAvailabilityOnStartup() {
   int availability_on_startup = g_browser_process->local_state()->GetInteger(
       prefs::kBrowserProfilePickerAvailabilityOnStartup);
@@ -273,3 +281,13 @@ void ProfilePicker::SetOpenCommandLineUrlsInNextProfileOpened(bool value) {
 bool ProfilePicker::GetOpenCommandLineUrlsInNextProfileOpened() {
   return g_open_command_line_urls_in_next_profile_opened;
 }
+
+// static
+void ProfilePicker::SetFirstRunTabsInNextProfileOpened(std::vector<GURL> urls) {
+  FirstRunTabsInNextProfileOpened() = std::move(urls);
+}
+
+// static
+std::vector<GURL> ProfilePicker::TakeFirstRunTabsInNextProfileOpened() {
+  return std::exchange(FirstRunTabsInNextProfileOpened(), std::vector<GURL>());
+}
