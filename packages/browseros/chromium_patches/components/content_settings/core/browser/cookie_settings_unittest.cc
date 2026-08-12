diff --git a/components/content_settings/core/browser/cookie_settings_unittest.cc b/components/content_settings/core/browser/cookie_settings_unittest.cc
index af3602048602eaccbd5b928b24003c750c6f015a..9a61bc67c85ef187454b81aa58c3576a44b1a1a1 100644
--- a/components/content_settings/core/browser/cookie_settings_unittest.cc
+++ b/components/content_settings/core/browser/cookie_settings_unittest.cc
@@ -611,6 +611,8 @@ TEST_P(CookieSettingsTestP, CookiesBlockThirdParty) {
 }
 
 TEST_F(CookieSettingsTest, CookiesControlsDefault) {
+  EXPECT_EQ(static_cast<int>(CookieControlsMode::kIncognitoOnly),
+            prefs_.GetInteger(prefs::kCookieControlsMode));
   EXPECT_TRUE(cookie_settings_->IsFullCookieAccessAllowed(
       kBlockedSite, kFirstPartySiteForCookies,
       /*top_frame_origin=*/std::nullopt, net::CookieSettingOverrides(),
