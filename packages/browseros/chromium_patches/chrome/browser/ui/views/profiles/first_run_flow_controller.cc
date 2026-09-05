diff --git a/chrome/browser/ui/views/profiles/first_run_flow_controller.cc b/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
index 24848513afc0ffa5649fbe690055d0d018c8d367..c8b51bbd69806159c0f1279daa73926bbaaa47c5 100644
--- a/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
+++ b/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
@@ -26,6 +26,9 @@
 #include "base/time/time.h"
 #include "base/version_info/channel.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_loader.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding_prefs.h"
 #include "chrome/browser/enterprise/util/managed_browser_utils.h"
 #include "chrome/browser/policy/cloud/user_policy_signin_service.h"
 #include "chrome/browser/policy/cloud/user_policy_signin_service_factory.h"
@@ -255,6 +258,66 @@ class IntroStepController : public ProfileManagementStepController {
   base::WeakPtrFactory<IntroStepController> weak_ptr_factory_{this};
 };
 
+class BrowserOSOnboardingStepController
+    : public ProfileManagementStepController {
+ public:
+  BrowserOSOnboardingStepController(ProfilePickerWebContentsHost* host,
+                                    base::RepeatingClosure completion_callback,
+                                    BrowserOSOnboardingEnsureReady ensure_ready)
+      : ProfileManagementStepController(host),
+        completion_callback_(std::move(completion_callback)),
+        ensure_ready_(std::move(ensure_ready)) {}
+
+  ~BrowserOSOnboardingStepController() override = default;
+
+  void Show(StepSwitchFinishedCallback step_shown_callback,
+            bool reset_state) override {
+    if (reset_state) {
+      host()->ShowScreenInPickerContents(
+          GURL(chrome::kChromeUIBrowserOSOnboardingURL),
+          base::BindOnce(&BrowserOSOnboardingStepController::OnLoaded,
+                         weak_ptr_factory_.GetWeakPtr(),
+                         std::move(step_shown_callback)));
+      return;
+    }
+
+    DCHECK_EQ(GURL(chrome::kChromeUIBrowserOSOnboardingURL),
+              host()->GetPickerContents()->GetURL());
+    host()->ShowScreenInPickerContents(
+        GURL(), base::BindOnce(std::move(step_shown_callback.value()), true));
+    ExpectCompletionCallback();
+  }
+
+  void OnNavigateBackRequested() override {
+    NavigateBackInternal(host()->GetPickerContents());
+  }
+
+ private:
+  void OnLoaded(StepSwitchFinishedCallback step_shown_callback) {
+    std::move(step_shown_callback.value()).Run(/*success=*/true);
+    ExpectCompletionCallback();
+  }
+
+  void ExpectCompletionCallback() {
+    auto* onboarding_ui = host()
+                              ->GetPickerContents()
+                              ->GetWebUI()
+                              ->GetController()
+                              ->GetAs<BrowserOSOnboarding>();
+    DCHECK(onboarding_ui);
+    onboarding_ui->SetCompletionCallback(completion_callback_, ensure_ready_,
+                                          setup_state_);
+  }
+
+  base::RepeatingClosure completion_callback_;
+  BrowserOSOnboardingEnsureReady ensure_ready_;
+  scoped_refptr<BrowserOSOnboardingSetupState> setup_state_ =
+      base::MakeRefCounted<BrowserOSOnboardingSetupState>();
+
+  base::WeakPtrFactory<BrowserOSOnboardingStepController> weak_ptr_factory_{
+      this};
+};
+
 class DefaultBrowserStepController : public ProfileManagementStepController {
  public:
   explicit DefaultBrowserStepController(
@@ -965,47 +1028,25 @@ void FirstRunFlowController::StartBrowsing() {
 void FirstRunFlowController::Init() {
   RegisterStep(
       Step::kIntro,
-      CreateIntroStep(
+      std::make_unique<BrowserOSOnboardingStepController>(
           host(),
-          base::BindRepeating(&FirstRunFlowController::HandleIntroSigninChoice,
-                              weak_ptr_factory_.GetWeakPtr()),
-          /*enable_animations=*/true,
-          base::BindRepeating(&FirstRunFlowController::AreEffectsEnabled,
-                              base::Unretained(this))));
+          base::BindRepeating(
+              &FirstRunFlowController::HandleBrowserOSOnboardingComplete,
+              weak_ptr_factory_.GetWeakPtr()),
+          base::BindRepeating(
+              [](base::WeakPtr<FirstRunFlowController> flow,
+                 base::OnceCallback<void(bool)> callback) {
+                if (!flow) {
+                  std::move(callback).Run(false);
+                  return;
+                }
+                // Picker contents can belong to a different profile. The
+                // extension handoff must wait for the actual browsing profile.
+                browseros::BrowserOSExtensionLoader::EnsurePrimaryExtensionReady(
+                    flow->profile_, std::move(callback));
+              },
+              weak_ptr_factory_.GetWeakPtr())));
   SwitchToStep(Step::kIntro, /*reset_state=*/true);
-
-  if (switches::IsFirstRunDesktopRevampEnabled(
-          IsProfileInSearchEngineChoiceRegion(profile_))) {
-    sounds_manager_ = GetSoundsManagerFactory().Run(
-        content::GetAudioServiceStreamFactoryBinder());
-    if (sounds_manager_) {
-      sounds_manager_->Initialize(kLogoSoundKey, IDR_INTRO_SOUND_LOGO_FLAC,
-                                  media::AudioCodec::kFLAC, /*loop=*/false);
-      sounds_manager_->Initialize(kAmbientSoundKey,
-                                  IDR_INTRO_SOUND_AMBIENT_FLAC,
-                                  media::AudioCodec::kFLAC, /*loop=*/true);
-      sounds_manager_->Initialize(kWelcomeBackSoundKey,
-                                  IDR_INTRO_SOUND_WELCOME_BACK_FLAC,
-                                  media::AudioCodec::kFLAC, /*loop=*/false);
-      sounds_manager_->Initialize(kFeatureShowcaseAmbientSoundKey,
-                                  IDR_INTRO_SOUND_FEATURE_SHOWCASE_AMBIENT_FLAC,
-                                  media::AudioCodec::kFLAC, /*loop=*/true);
-      sounds_manager_->Initialize(
-          kFeatureShowcaseProgressSoundKey,
-          IDR_INTRO_SOUND_FEATURE_SHOWCASE_PROGRESS_FLAC,
-          media::AudioCodec::kFLAC, /*loop=*/false);
-      sounds_manager_->Initialize(kAllSetSoundKey, IDR_INTRO_SOUND_ALL_SET_FLAC,
-                                  media::AudioCodec::kFLAC, /*loop=*/false);
-      if (AreEffectsEnabled()) {
-        sounds_manager_->Play(kLogoSoundKey);
-        sounds_manager_->Play(kAmbientSoundKey);
-      }
-    }
-  }
-
-  signin_metrics::LogSignInOffered(
-      kAccessPoint, signin_metrics::PromoAction::
-                        PROMO_ACTION_NEW_ACCOUNT_NO_EXISTING_ACCOUNT);
 }
 
 void FirstRunFlowController::CancelSigninFlow() {
@@ -1060,6 +1101,11 @@ void FirstRunFlowController::HandleIntroSigninChoice(IntroChoice choice) {
       kAccessPoint, profile_->GetPath());
 }
 
+void FirstRunFlowController::HandleBrowserOSOnboardingComplete() {
+  browseros::onboarding::MarkCompleted(profile_);
+  FinishFlowAndRunInBrowser(profile_, PostHostClearedCallback());
+}
+
 std::unique_ptr<ProfilePickerPostSignInAdapter>
 FirstRunFlowController::CreatePostSignInAdapter(
     Profile* signed_in_profile,
