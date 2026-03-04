diff --git a/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc b/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc
index 9ddd7c784c3f2..c80cff3197880 100644
--- a/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc
+++ b/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc
@@ -8,6 +8,7 @@
 
 #include "base/strings/string_number_conversions.h"
 #include "base/strings/to_string.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/sessions/session_service.h"
 #include "chrome/browser/sessions/session_service_factory.h"
@@ -43,6 +44,8 @@ VerticalTabStripStateController::VerticalTabStripStateController(
       browser_window_(browser_window),
       scoped_unowned_user_data_(browser_window->GetUnownedUserDataHost(),
                                 *this) {
+  browseros::SyncVerticalTabsPref(pref_service_);
+
   pref_change_registrar_.Init(pref_service_);
 
   pref_change_registrar_.Add(
@@ -50,6 +53,16 @@ VerticalTabStripStateController::VerticalTabStripStateController(
       base::BindRepeating(&VerticalTabStripStateController::NotifyStateChanged,
                           base::Unretained(this)));
 
+  pref_change_registrar_.Add(
+      browseros::prefs::kVerticalTabsEnabled,
+      base::BindRepeating(
+          [](PrefService* ps) {
+            ps->SetBoolean(
+                prefs::kVerticalTabsEnabled,
+                ps->GetBoolean(browseros::prefs::kVerticalTabsEnabled));
+          },
+          base::Unretained(pref_service_)));
+
   if (restored_state_collapsed.has_value()) {
     SetCollapsed(restored_state_collapsed.value());
   }
