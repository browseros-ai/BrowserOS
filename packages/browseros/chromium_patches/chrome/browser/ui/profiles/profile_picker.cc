diff --git a/chrome/browser/ui/profiles/profile_picker.cc b/chrome/browser/ui/profiles/profile_picker.cc
index 4d0b91c7a0b97..0f34b87dcda91 100644
--- a/chrome/browser/ui/profiles/profile_picker.cc
+++ b/chrome/browser/ui/profiles/profile_picker.cc
@@ -5,12 +5,15 @@
 #include "chrome/browser/ui/profiles/profile_picker.h"
 
 #include <string>
+#include <utility>
+#include <vector>
 
 #include "base/command_line.h"
 #include "base/containers/flat_set.h"
 #include "base/feature_list.h"
 #include "base/logging.h"
 #include "base/metrics/histogram_functions.h"
+#include "base/no_destructor.h"
 #include "chrome/browser/browser_process.h"
 #include "chrome/browser/profiles/profile_manager.h"
 #include "chrome/browser/profiles/profiles_state.h"
@@ -25,6 +28,11 @@ namespace {
 
 bool g_open_command_line_urls_in_next_profile_opened = false;
 
+std::vector<GURL>& FirstRunTabsInNextProfileOpened() {
+  static base::NoDestructor<std::vector<GURL>> first_run_tabs;
+  return *first_run_tabs;
+}
+
 ProfilePicker::AvailabilityOnStartup GetAvailabilityOnStartup() {
   int availability_on_startup = g_browser_process->local_state()->GetInteger(
       prefs::kBrowserProfilePickerAvailabilityOnStartup);
@@ -202,3 +210,13 @@ void ProfilePicker::SetOpenCommandLineUrlsInNextProfileOpened(bool value) {
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
