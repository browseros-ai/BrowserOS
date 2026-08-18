diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
index 4aa59aad615fbf2b376848d8f07bb56c14008aaf..37c69f73e07122ecbc58fe195d59c45d3c7d5534 100644
--- a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
+++ b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
@@ -287,7 +287,7 @@ TEST_F(CustomizeToolbarHandlerTest, PinForward) {
 }
 
 TEST_F(CustomizeToolbarHandlerTest, PinSplitTab) {
-  ASSERT_FALSE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
+  ASSERT_TRUE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
 
   handler().PinAction(side_panel::customize_chrome::mojom::ActionId::kSplitTab,
                       false);
