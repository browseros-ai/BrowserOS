diff --git a/components/bookmarks/browser/bookmark_utils.cc b/components/bookmarks/browser/bookmark_utils.cc
index d5beb2ca1188f..9e8764bf22776 100644
--- a/components/bookmarks/browser/bookmark_utils.cc
+++ b/components/bookmarks/browser/bookmark_utils.cc
@@ -450,7 +450,7 @@ bool DoesBookmarkContainWords(const std::u16string& title,
 
 void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(
-      prefs::kShowBookmarkBar, false,
+      prefs::kShowBookmarkBar, true,
       user_prefs::PrefRegistrySyncable::SYNCABLE_PREF);
   registry->RegisterBooleanPref(prefs::kEditBookmarksEnabled, true);
   registry->RegisterBooleanPref(
