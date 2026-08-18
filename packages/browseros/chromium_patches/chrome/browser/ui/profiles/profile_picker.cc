diff --git a/chrome/browser/ui/profiles/profile_picker.cc b/chrome/browser/ui/profiles/profile_picker.cc
index 1c1e093d40a288d65b981d4f5a30630430fa72f1..7a8787c8cca093ffc93a9f4b38f0d00f66b0e6a2 100644
--- a/chrome/browser/ui/profiles/profile_picker.cc
+++ b/chrome/browser/ui/profiles/profile_picker.cc
@@ -5,6 +5,8 @@
 #include "chrome/browser/ui/profiles/profile_picker.h"
 
 #include <string>
+#include <utility>
+#include <vector>
 
 #include "base/check_is_test.h"
 #include "base/command_line.h"
@@ -12,6 +14,7 @@
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
@@ -205,3 +213,13 @@ void ProfilePicker::SetOpenCommandLineUrlsInNextProfileOpened(bool value) {
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
