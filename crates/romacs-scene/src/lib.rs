//! The retained scene graph: the one drawing object Lisp hands the host.
//!
//! Lisp builds a tree of nodes, sends it once as `VM2GFX01` bytes
//! ([`wire`]), then sends transactional diffs ([`diff`], [`scene`]). The
//! host keeps every scene with its leases and pool share ([`host`]), lays a
//! placement out with taffy ([`layout`]), samples property motion on the
//! scene's document clock ([`motion`]), and lowers it to the renderer's
//! [`draw::DrawList`] ([`lower`]). Hit testing ([`hit`]) answers from what a
//! placement presented, through the same composition walk ([`walk`]).
//!
//! No GPU and no VM code lives here: the renderer consumes `draw`, the VM
//! produces `wire`, and fonts plug in through [`layout::Measure`].

pub mod budget;
pub mod diff;
pub mod draw;
pub mod fixtures;
pub mod graph;
pub mod hit;
pub mod host;
pub mod layout;
pub mod lower;
pub mod motion;
pub mod node;
pub mod prop;
pub mod scene;
pub mod walk;
pub mod wire;
