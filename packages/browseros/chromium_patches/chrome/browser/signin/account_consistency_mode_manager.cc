diff --git a/chrome/browser/signin/account_consistency_mode_manager.cc b/chrome/browser/signin/account_consistency_mode_manager.cc
index 7d4c893cbcd995c48a00631f9cbfd736fb188e63..a73b676afc3341c2dac3833804cbc9074ab2d2e3 100644
--- a/chrome/browser/signin/account_consistency_mode_manager.cc
+++ b/chrome/browser/signin/account_consistency_mode_manager.cc
@@ -57,8 +57,8 @@ bool IsBrowserSigninAllowedByCommandLine() {
         command_line->GetSwitchValueASCII(kAllowBrowserSigninArgument);
     return base::EqualsCaseInsensitiveASCII(allowBrowserSignin, "true");
   }
-  // If the commandline flag is not provided, the default is true.
-  return true;
+  // If the commandline flag is not provided, the default is false.
+  return false;
 }
 
 // Returns true if Desktop Identity Consistency can be enabled for this build
