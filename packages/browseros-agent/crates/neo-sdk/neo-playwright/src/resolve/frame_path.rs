//! The parent frames between the page and a target's frame.

use crate::probe::{ParsedSelector, ProbeTarget};
use crate::world::{IsolatedWorld, LinkedParent, Pin};

/// One parent frame of the target's frame: the parent's world, and how to find the
/// `<iframe>` there that holds the next frame down.
#[derive(Clone, Debug)]
pub(crate) struct Hop {
    pub(crate) parent: IsolatedWorld,
    pub(crate) iframe: HopIframe,
}

/// How to find a hop's `<iframe>` in its parent's world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HopIframe {
    /// Pinned by this op's own frame hop.
    Pin(Pin),
    /// A parent link's key (a ref jump landed below it): resolved by Neo's aria-ref engine.
    Key(String),
}

impl HopIframe {
    /// The iframe as a probe request's target in the parent's world.
    pub(crate) fn target(&self) -> Result<ProbeTarget, serde_json::Error> {
        match self {
            Self::Pin(pin) => Ok(ProbeTarget::Pin(*pin)),
            Self::Key(key) => ParsedSelector::frame_key(key).map(ProbeTarget::Chunk),
        }
    }
}

/// The parents of a target's frame, from the main frame down; empty in the main frame. An
/// action walks it to hit-check each parent's iframe and to place its point in the main
/// viewport, where all input goes.
#[derive(Clone, Debug, Default)]
pub(crate) struct FramePath {
    hops: Vec<Hop>,
}

impl FramePath {
    /// The path a ref jump lands on: the parent links recorded when the frame was reached.
    pub(crate) fn from_parents(parents: Vec<LinkedParent>) -> Self {
        Self {
            hops: parents
                .into_iter()
                .map(|parent| Hop {
                    parent: parent.world,
                    iframe: HopIframe::Key(parent.iframe_key),
                })
                .collect(),
        }
    }

    /// One more frame down.
    pub(crate) fn push(&mut self, hop: Hop) {
        self.hops.push(hop);
    }

    /// The hops, from the main frame down.
    pub(crate) fn hops(&self) -> &[Hop] {
        &self.hops
    }
}
