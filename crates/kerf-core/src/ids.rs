//! Stable, opaque identifiers for the things a project contains.
//!
//! IDs are monotonic `u64`s handed out by [`IdGen`]. They are stable across a
//! session and serialize as plain numbers, so a command that refers to "clip 7"
//! means the same thing whether it came from the UI or, later, an MCP agent.

use serde::{Deserialize, Serialize};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}#{}", stringify!($name), self.0)
            }
        }
    };
}

define_id!(MediaId);
define_id!(ClipId);
define_id!(TrackId);
define_id!(MarkerId);

/// Hands out fresh, never-reused ids for one project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdGen {
    next: u64,
}

impl Default for IdGen {
    fn default() -> Self {
        IdGen { next: 1 }
    }
}

impl IdGen {
    fn raw(&mut self) -> u64 {
        let v = self.next;
        self.next += 1;
        v
    }

    pub fn media(&mut self) -> MediaId {
        MediaId(self.raw())
    }
    pub fn clip(&mut self) -> ClipId {
        ClipId(self.raw())
    }
    pub fn track(&mut self) -> TrackId {
        TrackId(self.raw())
    }
    pub fn marker(&mut self) -> MarkerId {
        MarkerId(self.raw())
    }
    /// A fresh link-group number for clips that belong together.
    pub fn link(&mut self) -> u64 {
        self.raw()
    }
}
