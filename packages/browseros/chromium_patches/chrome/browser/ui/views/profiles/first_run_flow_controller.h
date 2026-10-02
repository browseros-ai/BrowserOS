diff --git a/chrome/browser/ui/views/profiles/first_run_flow_controller.h b/chrome/browser/ui/views/profiles/first_run_flow_controller.h
index e128130a918a09bd37062cd9d15c8d4ada1de394..d84a16d34a8adb699926bcd9a2d4df94496870b6 100644
--- a/chrome/browser/ui/views/profiles/first_run_flow_controller.h
+++ b/chrome/browser/ui/views/profiles/first_run_flow_controller.h
@@ -117,6 +117,9 @@ class FirstRunFlowController : public ProfileManagementFlowControllerImpl {
 
   void HandleIntroSigninChoice(IntroChoice choice);
 
+  // Invoked after the onboarding WebUI observes native extension readiness.
+  void HandleBrowserOSOnboardingComplete();
+
   void StartBrowsing();
 
   // Run the `finish_flow_callback_` if it's not empty.
