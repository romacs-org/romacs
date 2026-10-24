//! Runtime-independent contracts salvaged from the package scenario harness.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Terminal color capability shared by both editors in a package parity pair.
///
/// Keeping the supported profiles closed prevents a test from accidentally
/// changing GNU and Romacs through unrelated environment overrides.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalProfile {
    Indexed256,
    #[default]
    TrueColor,
}

/// Observable terminal-state contract for a package display checkpoint.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PackageDisplayContract {
    /// Compare visible text, geometry, cursor, faces, and resolved RGB colors.
    #[default]
    ExactDisplay,
    /// Compare the terminal's raw cell state, including indexed color values.
    RawTerminal,
}

/// Startup deadlines for a symmetric package TUI pair.
///
/// Most scenarios should use [`Self::same`]. A slower Romacs startup must be
/// declared explicitly at the call site instead of being hidden in the
/// harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairTimeout {
    Same(Duration),
    PerEditor { gnu: Duration, romacs: Duration },
}

impl PairTimeout {
    pub const fn same(timeout: Duration) -> Self {
        Self::Same(timeout)
    }

    pub const fn per_editor(gnu: Duration, romacs: Duration) -> Self {
        Self::PerEditor { gnu, romacs }
    }

    const fn split(self) -> (Duration, Duration) {
        match self {
            Self::Same(timeout) => (timeout, timeout),
            Self::PerEditor { gnu, romacs } => (gnu, romacs),
        }
    }
}

/// The startup observation that makes a spawned pair safe to drive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadinessCheckpoint {
    description: String,
    timeout: PairTimeout,
}

impl ReadinessCheckpoint {
    pub fn new(description: impl Into<String>, timeout: PairTimeout) -> Self {
        Self {
            description: description.into(),
            timeout,
        }
    }
}

/// A complete-display assertion selected deliberately by a package scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayCheckpoint {
    label: String,
    contract: PackageDisplayContract,
}

impl DisplayCheckpoint {
    /// Create the ordinary complete-screen contract: exact visible display
    /// parity after terminal colors have been resolved to RGB.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            contract: PackageDisplayContract::ExactDisplay,
        }
    }

    pub fn raw_terminal(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            contract: PackageDisplayContract::RawTerminal,
        }
    }
}

fn canonical_executable_identity(executable: &Path) -> Result<PathBuf, String> {
    let resolved = if executable.components().count() > 1 || executable.is_absolute() {
        executable.to_path_buf()
    } else {
        let path = env::var_os("PATH").ok_or_else(|| {
            format!("package TUI cannot resolve executable {executable:?}: PATH is absent")
        })?;
        env::split_paths(&path)
            .map(|directory| directory.join(executable))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                format!("package TUI cannot resolve executable {executable:?} through PATH")
            })?
    };
    fs::canonicalize(&resolved).map_err(|error| {
        format!(
            "package TUI cannot canonicalize editor executable {}: {error}",
            resolved.display()
        )
    })
}

fn validate_distinct_editor_identities(
    gnu: &Path,
    neo: &Path,
    allow_equal_for_expect_update: bool,
) -> Result<(), String> {
    if gnu == neo && !allow_equal_for_expect_update {
        return Err(format!(
            "package TUI GNU and Neo executables resolve to the same binary {}; set UPDATE_EXPECT=1 only for deliberate GNU snapshot calibration",
            gnu.display()
        ));
    }
    Ok(())
}

/// A validated display environment applied identically to both real editors.
///
/// The PTY owner already fixes `TERM`; package tests may only make `COLORTERM`
/// explicit or deliberately remove its inherited value.  One owned plan is
/// shared by both launch builders so the type prevents accidentally configuring
/// just one peer.
#[derive(Debug, Default, Eq, PartialEq)]
struct SymmetricDisplayEnvironment {
    set: Vec<(OsString, OsString)>,
    remove: Vec<OsString>,
}

impl From<TerminalProfile> for SymmetricDisplayEnvironment {
    fn from(profile: TerminalProfile) -> Self {
        match profile {
            TerminalProfile::Indexed256 => Self {
                set: Vec::new(),
                remove: vec![OsString::from("COLORTERM")],
            },
            TerminalProfile::TrueColor => Self {
                set: vec![(OsString::from("COLORTERM"), OsString::from("truecolor"))],
                remove: Vec::new(),
            },
        }
    }
}

impl SymmetricDisplayEnvironment {
    fn set_entries(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.set
            .iter()
            .map(|(key, value)| (key.as_os_str(), value.as_os_str()))
    }

    fn removed_entries(&self) -> impl Iterator<Item = &OsStr> {
        self.remove.iter().map(OsString::as_os_str)
    }
}

#[cfg(test)]
#[path = "tests/scenario_contract_test.rs"]
mod tests;
