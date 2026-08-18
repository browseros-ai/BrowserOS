diff --git a/chrome/browser/net/profile_network_context_service.cc b/chrome/browser/net/profile_network_context_service.cc
index c30ac8186a526d56b01e4deb87a747ac952e9d89..4c72849bea15c0740629041f70a425ccf7b409b6 100644
--- a/chrome/browser/net/profile_network_context_service.cc
+++ b/chrome/browser/net/profile_network_context_service.cc
@@ -647,7 +647,7 @@ void ProfileNetworkContextService::ConfigureNetworkContextParams(
 void ProfileNetworkContextService::RegisterProfilePrefs(
     user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(embedder_support::kAlternateErrorPagesEnabled,
-                                true);
+                                false);
   registry->RegisterBooleanPref(prefs::kQuicAllowed, true);
   registry->RegisterBooleanPref(prefs::kGloballyScopeHTTPAuthCacheEnabled,
                                 false);
