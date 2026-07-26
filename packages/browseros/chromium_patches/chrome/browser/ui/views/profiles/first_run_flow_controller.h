diff --git a/chrome/browser/ui/views/profiles/first_run_flow_controller.h b/chrome/browser/ui/views/profiles/first_run_flow_controller.h
index ec68e45018034..a2644df0d8430 100644
--- a/chrome/browser/ui/views/profiles/first_run_flow_controller.h
+++ b/chrome/browser/ui/views/profiles/first_run_flow_controller.h
@@ -61,6 +61,7 @@ class FirstRunFlowController : public ProfileManagementFlowControllerImpl {
 
  private:
   void HandleIntroSigninChoice(IntroChoice choice);
+  void HandleBrowserOSOnboardingComplete();
 
   // Run the `finish_flow_callback_` if it's not empty.
   void RunFinishFlowCallback();
