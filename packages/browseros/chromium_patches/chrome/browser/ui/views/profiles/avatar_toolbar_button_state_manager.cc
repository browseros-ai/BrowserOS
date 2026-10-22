diff --git a/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc b/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc
index a514402e94a08f13a364ea9340b17d9787bd3bb5..a1662b39c3713771735b8358704950dd7b45b3d5 100644
--- a/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc
+++ b/chrome/browser/ui/views/profiles/avatar_toolbar_button_state_manager.cc
@@ -2499,8 +2499,9 @@ void AvatarToolbarButtonStateManager::CreateStatesAndListeners(
             /*state_observer=*/this);
 
 #if BUILDFLAG(ENABLE_DICE_SUPPORT)
-    if (syncer::IsReplaceSyncPromosWithSignInPromosEnabled() ||
-        switches::IsAvatarSyncPromoFeatureEnabled()) {
+    if (AccountConsistencyModeManager::IsDiceEnabledForProfile(profile) &&
+        (syncer::IsReplaceSyncPromosWithSignInPromosEnabled() ||
+         switches::IsAvatarSyncPromoFeatureEnabled())) {
       auto promo_state_provider =
           std::make_unique<PromoStateProvider>(browser,
                                                /*state_observer=*/this);
