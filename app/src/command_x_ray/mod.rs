//! Command x-ray: hover (or ask) for a description of the token under the pointer in a shell
//! command.
//!
//! Command x-ray has three moving parts, and all three live here so that every host runs the same
//! code:
//!
//! * [`hover`] — the host-agnostic hover state machine: the 500ms delay, the 3px movement
//!   threshold, whether the tooltip is open, and whether the user dismissed it.
//! * [`host`] — the seam a host view implements (command text, completion context, where the
//!   description goes) plus the shared describe/show/hide flow built on top of it.
//! * [`tooltip`] — the single description card and the shared overlay anchoring.
//!
//! What is *not* shared is pointer geometry. Resolving a pixel position to a byte offset depends
//! on layout that only a host's own element has — soft wrap and notches in the terminal input's
//! editor, viewport-relative block layout in the code editor — so each host hit-tests with its own
//! machinery and feeds the result to the shared state machine as a [`hover::HoverProbe`].
//!
//! [`hover_area`] carries the one piece of plumbing a host cannot supply for itself when its text
//! is rendered by a child view: raw pointer positions and element bounds. The terminal input's
//! editor element already has both, so it drives the state machine directly and does not use it.

pub mod host;
pub mod hover;
pub mod hover_area;
pub mod tooltip;

pub use host::{CommandXRayContext, CommandXRayHost, CommandXRayUpdate};
pub use hover::{CommandXRayHover, HoverOutcome, HoverProbe};
pub use hover_area::CommandXRayHoverArea;
pub use tooltip::{CommandXRayTooltipAnchor, add_command_x_ray_overlay};

#[cfg(test)]
#[path = "hover_tests.rs"]
mod hover_tests;
