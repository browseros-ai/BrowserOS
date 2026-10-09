diff --git a/chrome/browser/ui/views/profiles/first_run_flow_controller.cc b/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
index e6a815e2c00d48131a0b3b24c51c1830dc67afe4..975a05f938d2d05d34411e99e31759f615c97397 100644
--- a/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
+++ b/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
@@ -25,6 +25,9 @@
 #include "base/time/time.h"
 #include "base/version_info/channel.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/extensions/browseros_extension_loader.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding_prefs.h"
 #include "chrome/browser/enterprise/util/managed_browser_utils.h"
 #include "chrome/browser/policy/cloud/user_policy_signin_service.h"
 #include "chrome/browser/policy/cloud/user_policy_signin_service_factory.h"
@@ -268,6 +271,70 @@ class IntroStepController : public ProfileManagementStepController {
   base::WeakPtrFactory<IntroStepController> weak_ptr_factory_{this};
 };
 
+// Owns native setup state across onboarding WebUI reloads. Completion stays
+// gated on the browsing profile's extension readiness, even when the picker
+// WebContents belongs to a different profile.
+class BrowserOSOnboardingStepController
+    : public ProfileManagementStepController {
+ public:
+  BrowserOSOnboardingStepController(
+      ProfilePickerWebContentsHost* host,
+      base::RepeatingClosure completion_callback,
+      BrowserOSOnboardingEnsureReady ensure_ready)
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
+                                        setup_state_);
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
@@ -1024,46 +1091,30 @@ void FirstRunFlowController::StartBrowsing() {
 }
 
 void FirstRunFlowController::Init() {
-  if (switches::IsFirstRunDesktopRevampEnabled(
-          IsProfileInSearchEngineChoiceRegion(profile_))) {
-    if (base::FeatureList::IsEnabled(switches::kFirstRunDesktopRevampSound)) {
-      sounds_manager_ = GetSoundsManagerFactory().Run(
-          content::GetAudioServiceStreamFactoryBinder());
-    }
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
-    }
-  }
-
-  if (switches::IsPreFirstRunDesktopRefreshEnabled()) {
-    RegisterStep(
-        Step::kWelcome,
-        std::make_unique<WelcomeStepController>(
-            host(), base::BindOnce(&FirstRunFlowController::OnWelcomeCompleted,
-                                   weak_ptr_factory_.GetWeakPtr())));
-    SwitchToStep(Step::kWelcome, /*reset_state=*/true);
-  } else {
-    RegisterAndSwitchToIntroStep(
-        /*effects_button_shown_by_default=*/sounds_manager_ != nullptr);
-  }
-
-  PlaySound(kAmbientSoundKey);
+  // BrowserOS owns first run, including import and extension setup. Enter its
+  // WebUI directly; Chromium's welcome/sign-in steps must not bypass the native
+  // readiness gate or replace this onboarding flow.
+  RegisterStep(
+      Step::kIntro,
+      std::make_unique<BrowserOSOnboardingStepController>(
+          host(),
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
+  SwitchToStep(Step::kIntro, /*reset_state=*/true);
 }
 
 void FirstRunFlowController::CancelSigninFlow() {
@@ -1130,6 +1181,13 @@ void FirstRunFlowController::HandleIntroSigninChoice(IntroChoice choice) {
       kAccessPoint, profile_->GetPath());
 }
 
+void FirstRunFlowController::HandleBrowserOSOnboardingComplete() {
+  // The WebUI posts this callback only after native setup reaches READY. Persist
+  // completion for the browsing profile before handing off to its window.
+  browseros::onboarding::MarkCompleted(profile_);
+  FinishFlowAndRunInBrowser(profile_, PostHostClearedCallback());
+}
+
 std::unique_ptr<ProfilePickerPostSignInAdapter>
 FirstRunFlowController::CreatePostSignInAdapter(
     Profile* signed_in_profile,
