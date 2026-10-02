diff --git a/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc b/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc
index 22ece00a8c729ef5ada1d20afa2e289b8fd95d16..6b57459d409edde59b652e8af8944e3b2c9a3e05 100644
--- a/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc
+++ b/chrome/browser/ui/tabs/vertical_tab_strip_state_controller.cc
@@ -12,6 +12,7 @@
 #include "base/strings/string_number_conversions.h"
 #include "base/strings/to_string.h"
 #include "base/timer/timer.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/app/vector_icons/vector_icons.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/sessions/session_service.h"
@@ -58,6 +59,8 @@ VerticalTabStripStateController::VerticalTabStripStateController(
       browser_window_(browser_window),
       scoped_unowned_user_data_(browser_window->GetUnownedUserDataHost(),
                                 *this) {
+  browseros::SyncVerticalTabsPref(pref_service_);
+
   pref_change_registrar_.Init(pref_service_);
 
   is_vertical_tabs_enabled_ =
@@ -79,6 +82,16 @@ VerticalTabStripStateController::VerticalTabStripStateController(
             base::Unretained(this)));
   }
 
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
