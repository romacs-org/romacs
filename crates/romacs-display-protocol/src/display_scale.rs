//! Validated facts reported by a native display frontend for frame scaling.
//!
//! This module deliberately records observations, not policy conclusions.
//! In particular, Xwayland is a server identity; whether its physical-size
//! report is useful belongs to the layout policy that consumes the report.

use std::num::NonZeroU32;
