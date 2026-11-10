//! What each pointer action waits for before it sends input (design D6), as one table, so the
//! `PointerRunner` has no per-action branches for waiting. Checked against upstream's
//! `_performPointerAction` and its callers in server/dom.ts and server/frames.ts at v1.64.1; a
//! difference is fixed in this table and nowhere else.

use crate::probe::{ElementState, HitAction, Retarget};

/// The pointer actions, each one of upstream's named actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerKind {
    /// `click`, and the click inside `check` and `uncheck`.
    Click,
    Dblclick,
    Hover,
    /// `dragTo`'s first half, upstream's `move and down` on the source.
    DragSource,
    /// `dragTo`'s second half, upstream's `move and up` on the target.
    DragTarget,
}

/// One row of D6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Requirements {
    /// Upstream's action name, which the call log prints (`attempting click action`).
    pub(crate) name: &'static str,
    /// What the hit test and the interceptor retarget to (`setupHitTargetInterceptor`).
    pub(crate) retarget: Retarget,
    /// Checked in the page on every attempt, unless `force`. Stability is not a state: Rust
    /// compares two measurements on its own clock.
    pub(crate) states: &'static [ElementState],
    /// The interceptor kind armed around the input, unless `force`.
    pub(crate) hit: HitAction,
}

impl Requirements {
    /// The call log's `waiting for element to be ...` and `element is ...` wording.
    pub(crate) fn waits_for(self) -> &'static str {
        if self.states.contains(&ElementState::Enabled) {
            "visible, enabled and stable"
        } else {
            "visible and stable"
        }
    }
}

/// How upstream names a state in `element is not visible`.
pub(crate) fn state_name(state: ElementState) -> &'static str {
    match state {
        ElementState::Visible => "visible",
        ElementState::Enabled => "enabled",
        ElementState::Editable => "editable",
    }
}

/// D6 for pointer actions. Every one waits until visible and stable, then hit-tests; only the
/// ones that press a button wait for enabled (upstream's `waitForEnabled`). A drop's
/// interceptor is upstream's `drag` kind: a hit test before the input and none during it,
/// because the dragged element stays under the pointer.
pub(crate) const fn requirements(kind: PointerKind) -> Requirements {
    use ElementState::{Enabled, Visible};
    match kind {
        PointerKind::Click => Requirements {
            name: "click",
            retarget: Retarget::ButtonLink,
            states: &[Visible, Enabled],
            hit: HitAction::Mouse,
        },
        PointerKind::Dblclick => Requirements {
            name: "dblclick",
            retarget: Retarget::ButtonLink,
            states: &[Visible, Enabled],
            hit: HitAction::Mouse,
        },
        PointerKind::Hover => Requirements {
            name: "hover",
            retarget: Retarget::ButtonLink,
            states: &[Visible],
            hit: HitAction::Hover,
        },
        PointerKind::DragSource => Requirements {
            name: "move and down",
            retarget: Retarget::ButtonLink,
            states: &[Visible],
            hit: HitAction::Mouse,
        },
        PointerKind::DragTarget => Requirements {
            name: "move and up",
            retarget: Retarget::ButtonLink,
            states: &[Visible],
            hit: HitAction::Drag,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{PointerKind, requirements};
    use crate::probe::{ElementState, HitAction};

    #[test]
    fn tests_that_only_pressing_actions_wait_for_enabled() {
        for (kind, enabled) in [
            (PointerKind::Click, true),
            (PointerKind::Dblclick, true),
            (PointerKind::Hover, false),
            (PointerKind::DragSource, false),
            (PointerKind::DragTarget, false),
        ] {
            let row = requirements(kind);
            assert_eq!(
                row.states.contains(&ElementState::Enabled),
                enabled,
                "{kind:?}"
            );
            assert!(row.states.contains(&ElementState::Visible), "{kind:?}");
        }
        assert_eq!(
            requirements(PointerKind::Click).waits_for(),
            "visible, enabled and stable"
        );
        assert_eq!(
            requirements(PointerKind::Hover).waits_for(),
            "visible and stable"
        );
    }

    #[test]
    fn tests_that_each_action_arms_upstreams_interceptor_kind() {
        assert_eq!(requirements(PointerKind::Hover).hit, HitAction::Hover);
        assert_eq!(requirements(PointerKind::DragTarget).hit, HitAction::Drag);
        for kind in [
            PointerKind::Click,
            PointerKind::Dblclick,
            PointerKind::DragSource,
        ] {
            assert_eq!(requirements(kind).hit, HitAction::Mouse, "{kind:?}");
        }
    }
}
