diff --git a/chrome/browser/net/profile_network_context_service.cc b/chrome/browser/net/profile_network_context_service.cc
index 67be696b30045eb76c78cfaefec83bdf9ba580d3..469752a2849157b63f449743093c1ff98f58ce16 100644
--- a/chrome/browser/net/profile_network_context_service.cc
+++ b/chrome/browser/net/profile_network_context_service.cc
@@ -654,7 +654,7 @@ void ProfileNetworkContextService::ConfigureNetworkContextParams(
 void ProfileNetworkContextService::RegisterProfilePrefs(
     user_prefs::PrefRegistrySyncable* registry) {
   registry->RegisterBooleanPref(embedder_support::kAlternateErrorPagesEnabled,
-                                true);
+                                false);
   registry->RegisterBooleanPref(prefs::kQuicAllowed, true);
   registry->RegisterBooleanPref(prefs::kGloballyScopeHTTPAuthCacheEnabled,
                                 false);
