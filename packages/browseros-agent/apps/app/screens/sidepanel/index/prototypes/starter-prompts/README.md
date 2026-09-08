# Starter prompts — Paper prototype

Question: how should people customize the three shortcuts shown when a new conversation starts?

[Paper design: BrowserOS · Custom starter prompts](https://app.paper.design/file/01M213WD4SPQWNYJ021VD8FNJE/1-0)

This is a throwaway design study. No production behavior is changed. Open `index.html` directly to compare the exported Paper layouts, or open the Paper file to inspect and comment on the editable layers.

| Variant | Entry point | Editor | Tradeoff |
| --- | --- | --- | --- |
| A — recommended starting point | Edit prompts above the list | Edit three prompt fields in place | Simplest form; longer prompts also make longer button labels |
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
- Save commits the edited set once. Cancel discards the draft. Reset defaults changes the draft and still requires Save.
- Keep prompt execution separate from edit controls. Opening, saving, resetting, or cancelling an editor must never send a message.
- Storage watchers can keep open extension surfaces consistent. Local profile storage does not imply syncing to another app, profile, or device.
- The prototype has no persistence or live mutations.

## Review and capture

Paper's native screenshot and PNG export returned black images in this session. Layouts were reviewed by exporting exact JSX from Paper and rendering it locally in Chromium. This caught missing text-color inheritance and two editor layouts that pushed footer controls beyond the panel edge; the Paper nodes were corrected and re-exported.

`A.html`, `B.html`, and `C.html` are static review copies. The `.paper.txt` files preserve the corresponding JSX exports as non-executable design evidence. These artifacts belong only on this prototype branch; implement the selected design in the existing component after feedback.
