diff --git a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h
index 9f7cbd0272c0a..84b696bf47bce 100644
--- a/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h
+++ b/chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h
@@ -55,6 +55,9 @@ class PinnedToolbarActionsModel : public KeyedService {
     // the above methods, this does include pref updates.
     virtual void OnActionsChanged() {}
 
+    // Called when toolbar label visibility pref changes.
+    virtual void OnLabelsVisibilityChanged() {}
+
    protected:
     virtual ~Observer() = default;
   };
@@ -96,6 +99,11 @@ class PinnedToolbarActionsModel : public KeyedService {
   // Search migrations are complete.
   void MaybeMigrateExistingPinnedStates();
 
+  // Ensures that certain actions are always pinned to the toolbar.
+  // This is called during initialization to ensure specific actions
+  // (like Third Party LLM) are always visible.
+  void EnsureAlwaysPinnedActions();
+
   // Returns the ordered list of pinned ActionIds.
   virtual const std::vector<actions::ActionId>& PinnedActionIds() const;
 
@@ -114,6 +122,14 @@ class PinnedToolbarActionsModel : public KeyedService {
 
   void UpdatePref(const std::vector<actions::ActionId>& updated_list);
 
+  // Called when a BrowserOS visibility pref changes.
+  // Re-evaluates which actions should be pinned and notifies observers.
+  void OnBrowserOSVisibilityPrefChanged();
+
+  // Called when the toolbar labels pref changes.
+  // Notifies observers so buttons can refresh their labels.
+  void OnBrowserOSLabelsPrefChanged();
+
   // Our observers.
   base::ObserverList<Observer>::Unchecked observers_;
 
