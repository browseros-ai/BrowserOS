diff --git a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.h b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.h
index 2e4d96e3d69ab..9e3bde9501ab9 100644
--- a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.h
+++ b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.h
@@ -11,6 +11,7 @@
 #include "base/memory/raw_ptr.h"
 #include "base/memory/weak_ptr.h"
 #include "base/time/time.h"
+#include "components/split_tabs/split_tab_visual_data.h"
 #include "ui/actions/actions.h"
 #include "ui/base/models/menu_model.h"
 #include "ui/views/view_class_properties.h"
@@ -62,11 +63,14 @@ class PinnedActionToolbarButtonMenuModel final : public ui::MenuModel {
   actions::ActionId GetActionIdAtForTesting(size_t index);
 
  private:
+  enum class ItemKind { kAction, kSeparator, kSplitTabCreate };
+
   struct Item {
     Item(Item&&);
     Item(actions::ActionId action_id,
          ItemType type,
          std::optional<ui::ElementIdentifier> unique_id = std::nullopt);
+    Item(int string_id, split_tabs::SplitTabLayout split_tab_layout);
     explicit Item(ItemType type);
     Item& operator=(Item&&);
     ~Item();
@@ -74,11 +78,17 @@ class PinnedActionToolbarButtonMenuModel final : public ui::MenuModel {
     actions::ActionId action_id = 0;
     ItemType type = TYPE_COMMAND;
     std::optional<ui::ElementIdentifier> unique_id;
+    ItemKind kind = ItemKind::kAction;
+    int string_id = 0;
+    split_tabs::SplitTabLayout split_tab_layout =
+        split_tabs::SplitTabLayout::kVertical;
   };
 
   // Adds menu items specific to the action item to the menu.
   void AddActionSpecificItems();
 
+  bool CanCreateSplitTab() const;
+
   actions::ActionItem* GetActionItemFor(actions::ActionId id) const;
 
   // Returns true if the toolbar button can be pinned and false otherwise.
