diff --git a/components/content_settings/core/browser/cookie_settings_unittest.cc b/components/content_settings/core/browser/cookie_settings_unittest.cc
index 54dab8169a5d4..e059eda7f01a5 100644
--- a/components/content_settings/core/browser/cookie_settings_unittest.cc
+++ b/components/content_settings/core/browser/cookie_settings_unittest.cc
@@ -641,6 +641,8 @@ TEST_P(CookieSettingsTestP, CookiesBlockThirdParty) {
 }
 
 TEST_F(CookieSettingsTest, CookiesControlsDefault) {
+  EXPECT_EQ(static_cast<int>(CookieControlsMode::kIncognitoOnly),
+            prefs_.GetInteger(prefs::kCookieControlsMode));
   EXPECT_TRUE(cookie_settings_->IsFullCookieAccessAllowed(
       kBlockedSite, kFirstPartySiteForCookies,
       /*top_frame_origin=*/std::nullopt, net::CookieSettingOverrides(),
