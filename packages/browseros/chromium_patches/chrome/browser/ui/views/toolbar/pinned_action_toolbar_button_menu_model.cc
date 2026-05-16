diff --git a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.cc b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.cc
index d8a69c7965a46..82d85cc4e5cae 100644
--- a/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.cc
+++ b/chrome/browser/ui/views/toolbar/pinned_action_toolbar_button_menu_model.cc
@@ -15,18 +15,22 @@
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/browser_actions.h"
+#include "chrome/browser/ui/browser_commands.h"
 #include "chrome/browser/ui/browser_element_identifiers.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/customize_chrome/side_panel_controller.h"
 #include "chrome/browser/ui/tabs/public/tab_features.h"
+#include "chrome/browser/ui/tabs/split_tab_metrics.h"
 #include "chrome/browser/ui/tabs/tab_strip_model.h"
 #include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h"
 #include "chrome/common/pref_names.h"
+#include "chrome/grit/generated_resources.h"
 #include "components/prefs/pref_service.h"
 #include "components/tabs/public/tab_interface.h"
 #include "ui/actions/action_id.h"
 #include "ui/actions/actions.h"
 #include "ui/base/interaction/element_identifier.h"
+#include "ui/base/l10n/l10n_util.h"
 #include "ui/menus/simple_menu_model.h"
 
 DEFINE_ELEMENT_IDENTIFIER_VALUE(kPinnedActionToolbarUnpinElementId);
@@ -84,6 +88,10 @@ ui::MenuModel::ItemType PinnedActionToolbarButtonMenuModel::GetTypeAt(
     return items_[index].type;
   }
 
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    return items_[index].type;
+  }
+
   return GetActionItemFor(items_[index].action_id)->GetChecked()
              ? TYPE_CHECK
              : items_[index].type;
@@ -104,6 +112,10 @@ std::u16string PinnedActionToolbarButtonMenuModel::GetLabelAt(
     return std::u16string();
   }
 
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    return l10n_util::GetStringUTF16(items_[index].string_id);
+  }
+
   return std::u16string(GetActionItemFor(items_[index].action_id)->GetText());
 }
 
@@ -121,6 +133,9 @@ bool PinnedActionToolbarButtonMenuModel::IsItemCheckedAt(size_t index) const {
   if (GetTypeAt(index) == TYPE_SEPARATOR) {
     return false;
   }
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    return false;
+  }
 
   return GetActionItemFor(items_[index].action_id)->GetChecked();
 }
@@ -134,6 +149,9 @@ ui::ImageModel PinnedActionToolbarButtonMenuModel::GetIconAt(
   if (GetTypeAt(index) == TYPE_SEPARATOR) {
     return ui::ImageModel();
   }
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    return ui::ImageModel();
+  }
 
   const ui::ImageModel& image =
       GetActionItemFor(items_[index].action_id)->GetImage();
@@ -156,6 +174,9 @@ bool PinnedActionToolbarButtonMenuModel::IsEnabledAt(size_t index) const {
   if (GetTypeAt(index) == TYPE_SEPARATOR) {
     return true;
   }
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    return CanCreateSplitTab();
+  }
   if (items_[index].action_id == kActionPinActionToToolbar ||
       items_[index].action_id == kActionUnpinActionFromToolbar) {
     const bool is_pinnable = IsPinnable();
@@ -176,7 +197,14 @@ bool PinnedActionToolbarButtonMenuModel::IsEnabledAt(size_t index) const {
 }
 
 bool PinnedActionToolbarButtonMenuModel::IsVisibleAt(size_t index) const {
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    return CanCreateSplitTab();
+  }
   if (GetTypeAt(index) == TYPE_SEPARATOR) {
+    if (action_id_ == kActionSplitTab && index > 0 &&
+        items_[index - 1].kind == ItemKind::kSplitTabCreate) {
+      return CanCreateSplitTab();
+    }
     return true;
   }
   const bool is_pinned = IsPinned();
@@ -201,6 +229,13 @@ void PinnedActionToolbarButtonMenuModel::ActivatedAt(size_t index) {
 void PinnedActionToolbarButtonMenuModel::ActivatedAt(size_t index,
                                                      int event_flags) {
   DCHECK(GetTypeAt(index) != TYPE_SEPARATOR);
+  if (items_[index].kind == ItemKind::kSplitTabCreate) {
+    chrome::NewSplitTab(browser_,
+                        split_tabs::SplitTabCreatedSource::kToolbarButton,
+                        items_[index].split_tab_layout);
+    return;
+  }
+
   auto action_id = items_[index].action_id;
   if (action_id == kActionPinActionToToolbar ||
       action_id == kActionUnpinActionFromToolbar) {
@@ -230,12 +265,28 @@ PinnedActionToolbarButtonMenuModel::Item::Item(
     ItemType type,
     std::optional<ui::ElementIdentifier> unique_id)
     : action_id(action_id), type(type), unique_id(unique_id) {}
-PinnedActionToolbarButtonMenuModel::Item::Item(ItemType type) : type(type) {}
+PinnedActionToolbarButtonMenuModel::Item::Item(
+    int string_id,
+    split_tabs::SplitTabLayout split_tab_layout)
+    : kind(ItemKind::kSplitTabCreate),
+      string_id(string_id),
+      split_tab_layout(split_tab_layout) {}
+PinnedActionToolbarButtonMenuModel::Item::Item(ItemType type)
+    : type(type), kind(ItemKind::kSeparator) {}
 PinnedActionToolbarButtonMenuModel::Item&
 PinnedActionToolbarButtonMenuModel::Item::operator=(Item&&) = default;
 PinnedActionToolbarButtonMenuModel::Item::~Item() = default;
 
 void PinnedActionToolbarButtonMenuModel::AddActionSpecificItems() {
+  if (action_id_ == kActionSplitTab) {
+    items_.emplace_back(IDS_SPLIT_TAB_CREATE_LEFT_RIGHT,
+                        split_tabs::SplitTabLayout::kVertical);
+    items_.emplace_back(IDS_SPLIT_TAB_CREATE_TOP_BOTTOM,
+                        split_tabs::SplitTabLayout::kHorizontal);
+    items_.emplace_back(TYPE_SEPARATOR);
+    return;
+  }
+
   if (!IsPinStateManagedByPrefs(action_id_)) {
     // If the action has child actions add those first followed by a separator.
     actions::ActionItem* action_item = GetActionItemFor(action_id_);
@@ -252,6 +303,15 @@ void PinnedActionToolbarButtonMenuModel::AddActionSpecificItems() {
   }
 }
 
+bool PinnedActionToolbarButtonMenuModel::CanCreateSplitTab() const {
+  TabStripModel* const tab_strip_model = browser_->GetTabStripModel();
+  if (!tab_strip_model) {
+    return false;
+  }
+  tabs::TabInterface* const active_tab = tab_strip_model->GetActiveTab();
+  return active_tab && !active_tab->IsSplit();
+}
+
 actions::ActionItem* PinnedActionToolbarButtonMenuModel::GetActionItemFor(
     actions::ActionId id) const {
   return actions::ActionManager::Get().FindAction(
