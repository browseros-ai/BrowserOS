diff --git a/chrome/installer/util/util_constants.cc b/chrome/installer/util/util_constants.cc
index ea766d884fbda0a4e8ee1e9c7493ae9b68b3a319..0bfa855d119057fc5a181b572f635d948a708aaf 100644
--- a/chrome/installer/util/util_constants.cc
+++ b/chrome/installer/util/util_constants.cc
@@ -53,6 +53,9 @@ const char kDmServerUrl[] = "dm-server-url";
 // Prevent installer from launching Chrome after a successful first install.
 const char kDoNotLaunchChrome[] = "do-not-launch-chrome";
 
+// Suppress the interactive install UI.
+const char kSilent[] = "silent";
+
 // Prevents installer from writing the Google Update key that causes Google
 // Update to launch Chrome after a first install.
 const char kDoNotRegisterForUpdateLaunch[] =
