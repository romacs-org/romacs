//! Host boundary lint fixture: scanned by `host_boundary_lint.rs`, never
//! compiled. Each expected violation is tagged with an expect comment.
mod live;
mod clean;
#[path = "elsewhere/placed.rs"]
mod placed;
#[cfg(test)]
mod hidden;
mod nested {
    mod inner;
}
include!("included.rs");
