diff --git a/chrome/browser/ui/views/profiles/first_run_flow_controller.cc b/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
index 41048ae6fbd36..3b2d6f6162b34 100644
--- a/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
+++ b/chrome/browser/ui/views/profiles/first_run_flow_controller.cc
@@ -19,6 +19,8 @@
 #include "base/time/time.h"
 #include "base/version_info/channel.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding.h"
+#include "chrome/browser/browseros/onboarding/browseros_onboarding_prefs.h"
 #include "chrome/browser/enterprise/util/managed_browser_utils.h"
 #include "chrome/browser/policy/cloud/user_policy_signin_service.h"
 #include "chrome/browser/policy/cloud/user_policy_signin_service_factory.h"
@@ -195,6 +197,60 @@ class IntroStepController : public ProfileManagementStepController {
   base::WeakPtrFactory<IntroStepController> weak_ptr_factory_{this};
 };
 
+class BrowserOSOnboardingStepController
+    : public ProfileManagementStepController {
+ public:
+  BrowserOSOnboardingStepController(ProfilePickerWebContentsHost* host,
+                                    base::RepeatingClosure completion_callback)
+      : ProfileManagementStepController(host),
+        completion_callback_(std::move(completion_callback)) {}
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
+    onboarding_ui->SetCompletionCallback(completion_callback_);
+  }
+
+  base::RepeatingClosure completion_callback_;
+
+  base::WeakPtrFactory<BrowserOSOnboardingStepController> weak_ptr_factory_{
+      this};
+};
+
 class DefaultBrowserStepController : public ProfileManagementStepController {
  public:
   explicit DefaultBrowserStepController(
@@ -539,16 +595,12 @@ void FirstRunFlowController::ShowSigninError(Profile* profile,
 void FirstRunFlowController::Init() {
   RegisterStep(
       Step::kIntro,
-      CreateIntroStep(
+      std::make_unique<BrowserOSOnboardingStepController>(
           host(),
-          base::BindRepeating(&FirstRunFlowController::HandleIntroSigninChoice,
-                              weak_ptr_factory_.GetWeakPtr()),
-          /*enable_animations=*/true));
+          base::BindRepeating(
+              &FirstRunFlowController::HandleBrowserOSOnboardingComplete,
+              weak_ptr_factory_.GetWeakPtr())));
   SwitchToStep(Step::kIntro, /*reset_state=*/true);
-
-  signin_metrics::LogSignInOffered(
-      kAccessPoint, signin_metrics::PromoAction::
-                        PROMO_ACTION_NEW_ACCOUNT_NO_EXISTING_ACCOUNT);
 }
 
 void FirstRunFlowController::CancelSigninFlow() {
@@ -598,6 +650,11 @@ void FirstRunFlowController::HandleIntroSigninChoice(IntroChoice choice) {
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
