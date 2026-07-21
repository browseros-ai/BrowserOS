diff --git a/components/search/ntp_features.cc b/components/search/ntp_features.cc
index d4e14e9f1cfe32f8e5f5f53b5333242f7429e12b..0a226fe82a25c25dac171cc7a911ae2fdb9c70ab 100644
--- a/components/search/ntp_features.cc
+++ b/components/search/ntp_features.cc
@@ -238,7 +238,7 @@ BASE_FEATURE(kNtpStarterChip, base::FEATURE_DISABLED_BY_DEFAULT);
 BASE_FEATURE(kNtpOneGoogleBarAsyncBarParts, base::FEATURE_DISABLED_BY_DEFAULT);
 
 // If enabled, a footer will show on the NTP.
-BASE_FEATURE(kNtpFooter, base::FEATURE_ENABLED_BY_DEFAULT);
+BASE_FEATURE(kNtpFooter, base::FEATURE_DISABLED_BY_DEFAULT);
 
 // If enabled, tab groups module will be shown.
 BASE_FEATURE(kNtpTabGroupsModule,
