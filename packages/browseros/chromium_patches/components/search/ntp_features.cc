diff --git a/components/search/ntp_features.cc b/components/search/ntp_features.cc
index f5dba9a1dbb984282c53ceb05d75347a2efe5273..b4993a5e5877248d2999cc4a6f3d33c599d8a716 100644
--- a/components/search/ntp_features.cc
+++ b/components/search/ntp_features.cc
@@ -255,7 +255,7 @@ BASE_FEATURE(kNtpStarterChip, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE(kNtpOneGoogleBarAsyncBarParts, base::FEATURE_DISABLED_BY_DEFAULT);
 
 // If enabled, a footer will show on the NTP.
-BASE_FEATURE(kNtpFooter, base::FEATURE_ENABLED_BY_DEFAULT);
+BASE_FEATURE(kNtpFooter, base::FEATURE_DISABLED_BY_DEFAULT);
 
 // If enabled, tab groups module will be shown.
 BASE_FEATURE(kNtpTabGroupsModule,
