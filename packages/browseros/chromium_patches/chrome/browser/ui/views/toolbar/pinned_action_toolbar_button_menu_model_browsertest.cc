diff --git a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model_browsertest.cc b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model_browsertest.cc
index f03e5448c7f3c..e7948cf559bf7 100644
--- a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model_browsertest.cc
+++ b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model_browsertest.cc
@@ -6,12 +6,19 @@
 
 #include <string>
 
+#include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/browser_actions.h"
+#include "chrome/browser/ui/browser_commands.h"
+#include "chrome/browser/ui/tabs/split_tab_metrics.h"
+#include "chrome/grit/generated_resources.h"
 #include "chrome/test/base/in_process_browser_test.h"
+#include "components/split_tabs/split_tab_visual_data.h"
+#include "components/tabs/public/split_tab_data.h"
 #include "components/vector_icons/vector_icons.h"
 #include "content/public/test/browser_test.h"
 #include "ui/actions/actions.h"
+#include "ui/base/l10n/l10n_util.h"
 #include "ui/base/models/menu_model.h"
 #include "ui/menus/simple_menu_model.h"
 
@@ -96,3 +103,62 @@ IN_PROC_BROWSER_TEST_F(PinnedActionToolbarButtonMenuModelBrowserTest,
   EXPECT_TRUE(menu_model.IsEnabledAt(2));
   EXPECT_TRUE(menu_model.IsVisibleAt(2));
 }
+
+IN_PROC_BROWSER_TEST_F(PinnedActionToolbarButtonMenuModelBrowserTest,
+                       SplitTabCreateItems) {
+  PinnedActionToolbarButtonMenuModel menu_model(browser(), kActionSplitTab);
+
+  ASSERT_EQ(6u, menu_model.GetItemCount());
+  EXPECT_EQ(l10n_util::GetStringUTF16(IDS_SPLIT_TAB_CREATE_LEFT_RIGHT),
+            menu_model.GetLabelAt(0));
+  EXPECT_EQ(l10n_util::GetStringUTF16(IDS_SPLIT_TAB_CREATE_TOP_BOTTOM),
+            menu_model.GetLabelAt(1));
+  EXPECT_TRUE(menu_model.IsVisibleAt(0));
+  EXPECT_TRUE(menu_model.IsVisibleAt(1));
+  EXPECT_TRUE(menu_model.IsVisibleAt(2));
+}
+
+IN_PROC_BROWSER_TEST_F(PinnedActionToolbarButtonMenuModelBrowserTest,
+                       SplitTabCreateItemsHiddenWhenActiveTabIsSplit) {
+  chrome::NewSplitTab(browser(),
+                      split_tabs::SplitTabCreatedSource::kToolbarButton);
+  PinnedActionToolbarButtonMenuModel menu_model(browser(), kActionSplitTab);
+
+  ASSERT_EQ(6u, menu_model.GetItemCount());
+  EXPECT_FALSE(menu_model.IsVisibleAt(0));
+  EXPECT_FALSE(menu_model.IsVisibleAt(1));
+  EXPECT_FALSE(menu_model.IsVisibleAt(2));
+}
+
+IN_PROC_BROWSER_TEST_F(PinnedActionToolbarButtonMenuModelBrowserTest,
+                       SplitTabCreateLeftRightCreatesVerticalSplit) {
+  PinnedActionToolbarButtonMenuModel menu_model(browser(), kActionSplitTab);
+
+  menu_model.ActivatedAt(0);
+
+  auto split_id = browser()->tab_strip_model()->GetActiveTab()->GetSplit();
+  ASSERT_TRUE(split_id.has_value());
+  split_tabs::SplitTabVisualData* visual_data =
+      browser()
+          ->tab_strip_model()
+          ->GetSplitData(split_id.value())
+          ->visual_data();
+  EXPECT_EQ(split_tabs::SplitTabLayout::kVertical, visual_data->split_layout());
+}
+
+IN_PROC_BROWSER_TEST_F(PinnedActionToolbarButtonMenuModelBrowserTest,
+                       SplitTabCreateTopBottomCreatesHorizontalSplit) {
+  PinnedActionToolbarButtonMenuModel menu_model(browser(), kActionSplitTab);
+
+  menu_model.ActivatedAt(1);
+
+  auto split_id = browser()->tab_strip_model()->GetActiveTab()->GetSplit();
+  ASSERT_TRUE(split_id.has_value());
+  split_tabs::SplitTabVisualData* visual_data =
+      browser()
+          ->tab_strip_model()
+          ->GetSplitData(split_id.value())
+          ->visual_data();
+  EXPECT_EQ(split_tabs::SplitTabLayout::kHorizontal,
+            visual_data->split_layout());
+}
