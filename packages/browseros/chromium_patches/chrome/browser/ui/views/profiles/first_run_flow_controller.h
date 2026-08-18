diff --git a/chrome/browser/ui/views/profiles/first_run_flow_controller.h b/chrome/browser/ui/views/profiles/first_run_flow_controller.h
index 237d314f225405c39bede1efe56736a68043ea5c..c7b8afb1c26e57d39ccd5d23950331f73c21e8cf 100644
--- a/chrome/browser/ui/views/profiles/first_run_flow_controller.h
+++ b/chrome/browser/ui/views/profiles/first_run_flow_controller.h
@@ -112,6 +112,7 @@ class FirstRunFlowController : public ProfileManagementFlowControllerImpl {
   bool is_feature_showcase_eligible() const;
 
   void HandleIntroSigninChoice(IntroChoice choice);
+  void HandleBrowserOSOnboardingComplete();
 
   void PlaySignInCelebrationSound();
 
