diff --git a/chrome/browser/ui/toolbar/toolbar_pref_names.h b/chrome/browser/ui/toolbar/toolbar_pref_names.h
index d4ac82cdb34b109a991b4b31ef484a96a59b64e5..11f9285921a5d5c1108c92cd01a812547813c685 100644
--- a/chrome/browser/ui/toolbar/toolbar_pref_names.h
+++ b/chrome/browser/ui/toolbar/toolbar_pref_names.h
@@ -33,6 +33,10 @@ inline constexpr char kPinnedCastMigrationComplete[] =
 inline constexpr char kTabsFromOtherDevicesAutoPinnedMigration[] =
     "toolbar.tabs_from_other_devices_auto_pinned_migration";
 
+// Indicates whether Third Party LLM has been migrated to the new toolbar container.
+inline constexpr char kPinnedThirdPartyLlmMigrationComplete[] =
+    "toolbar.pinned_third_party_llm_migration_complete";
+
 }  // namespace prefs
 
 namespace toolbar {
