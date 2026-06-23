diff --git a/chrome/browser/ui/toolbar/toolbar_pref_names.h b/chrome/browser/ui/toolbar/toolbar_pref_names.h
index c59d5f14f663d..12da2438a579a 100644
--- a/chrome/browser/ui/toolbar/toolbar_pref_names.h
+++ b/chrome/browser/ui/toolbar/toolbar_pref_names.h
@@ -33,6 +33,10 @@ inline constexpr char kPinnedCastMigrationComplete[] =
 inline constexpr char kTabSearchMigrationComplete[] =
     "toolbar.tab_search_migration_complete";
 
+// Indicates whether Third Party LLM has been migrated to the new toolbar container.
+inline constexpr char kPinnedThirdPartyLlmMigrationComplete[] =
+    "toolbar.pinned_third_party_llm_migration_complete";
+
 }  // namespace prefs
 
 namespace toolbar {
