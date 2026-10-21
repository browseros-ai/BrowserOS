diff --git a/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc b/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc
index a514402e94a08f13a364ea9340b17d9787bd3bb5..beb6e7fd3d65da696f2c4d30d180bdd66cf7b09b 100644
--- a/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc
+++ b/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc
@@ -2499,8 +2499,13 @@ void AvatarToolbarButtonStateManager::CreateStatesAndListeners(
             /*state_observer=*/this);
 
 #if BUILDFLAG(ENABLE_DICE_SUPPORT)
-    if (syncer::IsReplaceSyncPromosWithSignInPromosEnabled() ||
-        switches::IsAvatarSyncPromoFeatureEnabled()) {
+    // The pill promos ("Sign in to BrowserOS?", sync, batch upload) all lead
+    // to browser sign-in, which is impossible when DICE is disabled for the
+    // profile (sign-in disallowed, e.g. BrowserOS neo). Account consistency is
+    // fixed for the profile's lifetime, so checking once here is sufficient.
+    if (AccountConsistencyModeManager::IsDiceEnabledForProfile(profile) &&
+        (syncer::IsReplaceSyncPromosWithSignInPromosEnabled() ||
+         switches::IsAvatarSyncPromoFeatureEnabled())) {
       auto promo_state_provider =
           std::make_unique<PromoStateProvider>(browser,
                                                /*state_observer=*/this);
