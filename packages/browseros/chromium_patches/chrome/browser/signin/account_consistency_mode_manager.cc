diff --git a/chrome/browser/signin/account_consistency_mode_manager.cc b/chrome/browser/signin/account_consistency_mode_manager.cc
index 7d4c893cbcd995c48a00631f9cbfd736fb188e63..7a9c8d06d49456d0f239ca3bd989b6d06be774dd 100644
--- a/chrome/browser/signin/account_consistency_mode_manager.cc
+++ b/chrome/browser/signin/account_consistency_mode_manager.cc
@@ -13,6 +13,7 @@
 #include "base/strings/string_util.h"
 #include "build/build_config.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/profiles/profile_attributes_storage.h"
 #include "chrome/browser/profiles/profile_manager.h"
@@ -169,6 +170,14 @@ bool AccountConsistencyModeManager::IsDiceEnabledForProfile(Profile* profile) {
 // static
 bool AccountConsistencyModeManager::IsDiceSignInAllowed(
     ProfileAttributesEntry* entry) {
+  // BrowserClaw (BrowserOS neo) has no browser account. This predicate feeds
+  // the per-launch kSigninAllowed pref (DICE, profile/app menu, web sign-in
+  // interception) and the profile picker / settings sign-in controls, so
+  // returning false here removes every browser sign-in entry point.
+  if (browseros::IsBrowserClawProduct()) {
+    return false;
+  }
+
   // Sign in should only be allowed for OIDC profiles with 3P identities that
   // are sync-ed to Google. Otherwise, we won't have a valid GAIA ID to sign in
   // to.
