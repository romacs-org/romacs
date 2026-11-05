//! Attachments' outgoing copies, made off the evaluator by the
//! `romacs-attach-tool` helper (design: attachments.md §2.4): `prepare`
//! converts, downscales and strips with the display's own decoder chain
//! (romacs-imaging), `blobs` moves a transcript's base64 images into a
//! session's store, `sweep` bounds the attach cache.  The store is
//! content-addressed and written by atomic rename.

pub mod blobs;
pub mod prepare;
pub mod store;
pub mod strip;
pub mod sweep;
