# Starter prompts — Paper prototype

Question: how should people customize the three shortcuts shown when a new conversation starts?

[Paper design: BrowserOS · Custom starter prompts](https://app.paper.design/file/01M213WD4SPQWNYJ021VD8FNJE/1-0)

This is a throwaway design study. No production behavior is changed. Open `index.html` directly to compare the exported Paper layouts, or open the Paper file to inspect and comment on the editable layers.

## Final direction from Paper feedback

The user chose C's “Customize prompts” link, revealed only when hovering near the prompts, and B's inline expansion. The **Final · Hover to customize, edit inline** artboard combines those decisions and shows three states together.

- At rest, show the three short labels and keep the customization link hidden.
- Hover anywhere in the prompt group to reveal the link. Reveal it on keyboard focus too; on devices without hover, keep a visible way to customize.
- Reserve the link's layout space so revealing it never shifts the list.
- Clicking Customize opens the first prompt inline. Editing the other rows expands them in place; none of these actions runs a prompt.
- Keep separate button-label and full-prompt fields.
- Keep draft edits while switching rows. Save prompts commits the entire set and returns to the normal view; Cancel or the editor's close action discards the draft.

The Save action sits below all three rows to make its scope clear. Local storage and mode-sharing decisions below remain implementation details to settle after final design review.

Feedback addressed:

- C: “like this customise prompts thing. but it should only com e up when hovering near the prompts?”
- B: “this inlide expand is good”

The original A/B/C studies remain for comparison. `Final.html` and `Final.paper.txt` capture the merged direction; the local viewer opens Final by default.

| Variant | Entry point | Editor | Tradeoff |
| --- | --- | --- | --- |
| A | Edit prompts above the list | Edit three prompt fields in place | Simplest form; longer prompts also make longer button labels |
| B | Pencil beside each prompt | Edit that shortcut's label and full prompt below its row | Fast individual edits; adds three controls beside the run actions |
| C | Customize prompts below the list | Dedicated pane with three slots, label, prompt, and preview | More room for detailed instructions; adds navigation |

Every study shows the normal and editing states together. The examples replace the promotional starter text with common browsing tasks to make the customization flow easier to judge; final default copy is still a decision.

## Existing integration points

- `../../ChatEmptyState.tsx` renders the current hard-coded suggestions.
- `modules/chat/chat-types.ts` owns separate Chat and Agent defaults, with `display`, `prompt`, and `icon`.
- Both side-panel chat and `screens/newtab/index/NewTabChat.tsx` use that empty-state component.
- Clicking a suggestion currently sends its full prompt immediately. Editing must remain a separate action.
- The app already uses WXT local extension storage for preferences, for example `lib/theme/theme-storage.ts`.

## Proposed behavior for implementation, pending feedback

- Exactly three slots, backed by local extension storage and shared across side-panel and new-tab chat in the same browser profile.
- Keep separate Chat and Agent sets unless the user chooses one shared set. This question remains open.
- Save commits the edited set once. Cancel discards the draft.
- Keep prompt execution separate from edit controls. Opening, saving, or cancelling an editor must never send a message.
- Storage watchers can keep open extension surfaces consistent. Local profile storage does not imply syncing to another app, profile, or device.
- The prototype has no persistence or live mutations.

## Review and capture

Paper's native screenshot and PNG export returned black images in this session. Layouts were reviewed by exporting exact JSX from Paper and rendering it locally in Chromium. This caught missing text-color inheritance and two editor layouts that pushed footer controls beyond the panel edge; the Paper nodes were corrected and re-exported.

The native screenshot path worked during the merged-final pass. The final Paper screenshot was reviewed for spacing, text contrast, alignment, and fit. The artboard uses content-driven height so its interaction notes remain visible.

`A.html`, `B.html`, and `C.html` are static review copies. The `.paper.txt` files preserve the corresponding JSX exports as non-executable design evidence. These artifacts belong only on this prototype branch; implement the selected design in the existing component after feedback.
