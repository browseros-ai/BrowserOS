diff --git a/chrome/browser/devtools/protocol/target_handler.cc b/chrome/browser/devtools/protocol/target_handler.cc
index f03b44a4953b7c1e05a3b062e1c8006504a572b1..1929465710681d8f672b8bceece9e08a4f12c1f2 100644
--- a/chrome/browser/devtools/protocol/target_handler.cc
+++ b/chrome/browser/devtools/protocol/target_handler.cc
@@ -120,6 +120,17 @@ protocol::Response TargetHandler::CreateTarget(
     DCHECK(profile);
   }
 
+  // Explicit regular contexts are now discoverable without opening a window
+  // through the profile picker. Enforce its lock/policy checks here as well as
+  // in Browser.createWindow; Navigate's creation-status check omits the lock.
+  if (browser_context_id.has_value() && profile->IsRegularProfile() &&
+      !profile->IsOffTheRecord() &&
+      !DevToolsBrowserContextManager::GetInstance()
+           .CanCreateRegularProfileWindow(profile)) {
+    return protocol::Response::ServerError(
+        "Profile policy does not allow a regular window");
+  }
+
   bool create_new_window = new_window.value_or(false);
   const bool should_focus = focus.value_or(!background.value_or(false));
   const bool create_in_background =
