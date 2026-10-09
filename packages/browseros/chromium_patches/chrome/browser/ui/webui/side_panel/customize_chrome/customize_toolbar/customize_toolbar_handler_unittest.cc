diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
index b4d6b14c4aa91c0f038a310077681737f6731097..8fb650a2bc22a481516bc9e34ed8cec029fb449e 100644
--- a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
+++ b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
@@ -227,7 +227,7 @@ TEST_F(CustomizeToolbarHandlerTest, PinForward) {
 }
 
 TEST_F(CustomizeToolbarHandlerTest, PinSplitTab) {
-  ASSERT_FALSE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
+  ASSERT_TRUE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
 
   handler().PinAction(side_panel::customize_chrome::mojom::ActionId::kSplitTab,
                       false);
