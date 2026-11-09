//! Native 3D model view (campaign design).
//!
//! `obj` and `scene` are the pure CPU side: OBJ parsing with the exact
//! semantics stored Gnosis data depends on, and scene normalisation.
//! `loader` runs them on a thread. `gpu` uploads a scene, draws it into a
//! sampled surface texture and answers picks. `store` is the render-thread
//! model state (requests, readiness, interaction) over [`ModelGpuHost`],
//! with `readiness` its level-triggered fence and `budget` its GPU pool.

pub mod budget;
pub mod gpu;
pub mod loader;
pub mod obj;
pub mod readiness;
pub mod scene;
pub mod store;

pub use gpu::{ModelDraw, ModelView, PickPoll, PickRequest};
#[cfg(test)]
pub(crate) use obj::read_obj_file;
pub use scene::ModelScene;
pub use store::{GlyphHit, ModelGpuHost, ModelStatus, ModelStore};
