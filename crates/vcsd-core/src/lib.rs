//! # VC Sound Design — core engine
//!
//! Sound-design-first DAW core: picture-synchronised timeline, layered
//! spotting, sound identities, cue chart, placeholder population, Foley
//! performance and recording, offline mixing, reports and studio handoff.
//!
//! The crate is UI-agnostic: every operation is a function over
//! [`state::ProjectState`], wrapped by [`state::Session::edit`] for
//! undo/redo and auditing. Timeline truth is integer samples.

pub mod builder;
pub mod editing;
pub mod error;
pub mod hash;
pub mod ids;
pub mod layout;
pub mod library;
pub mod mixer;
pub mod model;
pub mod perform;
pub mod picture;
pub mod spotting;
pub mod state;
pub mod store;
pub mod timecode;
pub mod wav;

pub use error::{Error, Result};
pub use model::*;
pub use state::{NewProject, ProjectState, Session};
pub use timecode::{FrameRate, TimeBase};
pub mod ai;
pub mod export;
pub mod handoff;
pub mod integrations;
pub mod pdf;
pub mod qa;
pub mod relink;
pub mod report;
pub mod synth;
pub mod transport;
