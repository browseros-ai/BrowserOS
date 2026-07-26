diff --git a/chrome/browser/net/profile_network_context_service.cc b/chrome/browser/net/profile_network_context_service.cc
index 7fed54f2fd078..34099919961da 100644
--- a/chrome/browser/net/profile_network_context_service.cc
+++ b/chrome/browser/net/profile_network_context_service.cc
@@ -626,7 +626,7 @@ void ProfileNetworkContextService::ConfigureNetworkContextParams(
 void ProfileNetworkContextService::RegisterProfilePrefs(
     user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(embedder_support::kAlternateErrorPagesEnabled,
-                                true);
+                                false);
   registry->RegisterBooleanPref(prefs::kQuicAllowed, true);
   registry->RegisterBooleanPref(prefs::kGloballyScopeHTTPAuthCacheEnabled,
                                 false);
