diff --git a/chrome/browser/ui/bookmarks/bookmark_bar_controller.cc b/chrome/browser/ui/bookmarks/bookmark_bar_controller.cc
index 68287f6eb78bf85479a488bee3be414657db072a..68a6097cd7e59f93735d9eeafea2f2fce4b1735d 100644
--- a/chrome/browser/ui/bookmarks/bookmark_bar_controller.cc
+++ b/chrome/browser/ui/bookmarks/bookmark_bar_controller.cc
@@ -7,6 +7,7 @@
 #include "base/feature_list.h"
 #include "base/types/to_address.h"
 #include "chrome/browser/bookmarks/bookmark_model_factory.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/defaults.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/search/search.h"
@@ -78,7 +79,16 @@ BookmarkBarController::BookmarkBarController(BrowserWindowInterface& browser,
   // Set up preference observer for bookmark bar visibility.
   Profile* profile = browser_->GetProfile();
   PrefService* prefs = profile->GetPrefs();
+  // Seed Chromium's pref before this controller computes its initial state.
+  // Existing upstream user choices win; later BrowserOS setting changes
+  // explicitly update the upstream pref observed below. The window-scoped
+  // registrar removes this callback before the profile's prefs are destroyed.
+  browseros::SyncShowTabGroupsInBookmarkBarPref(prefs);
   pref_change_registrar_.Init(prefs);
+  pref_change_registrar_.Add(
+      browseros::prefs::kShowTabGroupsInBookmarkBar,
+      base::BindRepeating(&browseros::ApplyShowTabGroupsInBookmarkBarPref,
+                          base::Unretained(prefs)));
   pref_change_registrar_.Add(
       bookmarks::prefs::kShowBookmarkBar,
       base::BindRepeating(&BookmarkBarController::UpdateBookmarkBarState,
