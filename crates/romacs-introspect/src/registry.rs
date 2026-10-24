//! What the runtimes actually register, as plain owned data.
//!
//! Filled either from `data/registry.json` (the committed snapshot) or, in a
//! process that links the runtimes, directly by `romacs-introspect-live`. There
//! is one JSON form and one reader for it, so the snapshot and a live
//! enumeration cannot be interpreted differently.

use serde_json::{Value as Json, json};
use std::collections::BTreeSet;
use std::fmt;

/// The most arguments a subr accepts, exactly as `subr-arity` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MaxArity {
    Fixed(u16),
    /// `&rest`: `subr-arity` says `many`.
    Many,
    /// A special form: arguments are not evaluated.
    Unevalled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Arity {
    pub min: u16,
    pub max: MaxArity,
}

impl fmt::Display for Arity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.max {
            MaxArity::Fixed(n) => write!(f, "({} . {})", self.min, n),
            MaxArity::Many => write!(f, "({} . many)", self.min),
            MaxArity::Unevalled => write!(f, "({} . unevalled)", self.min),
        }
    }
}

impl Arity {
    pub fn parse_max(s: &str) -> Option<MaxArity> {
        match s {
            "many" => Some(MaxArity::Many),
            "unevalled" => Some(MaxArity::Unevalled),
            n => n.parse().ok().map(MaxArity::Fixed),
        }
    }
    pub fn max_json(&self) -> Json {
        match self.max {
            MaxArity::Fixed(n) => json!(n),
            MaxArity::Many => json!("many"),
            MaxArity::Unevalled => json!("unevalled"),
        }
    }
}

/// One subr registered in `romacs-core`, as read from a booted `Context`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CoreSubr {
    /// The symbol whose function cell holds the subr.
    pub name: String,
    /// `subr-name` of that subr. Differs from `name` for an alias.
    pub subr_name: String,
    pub arity: Arity,
}

/// How a registry was obtained and when. Never compared for equality: two
/// enumerations of the same runtime at different times are the same registry.
#[derive(Clone, Debug, Default)]
pub struct RegistryProvenance {
    pub generated_by: String,
    /// Runtime-only enumeration time (legacy snapshots may supply it).
    /// Never serialized into the deterministic schema-v2 registry.
    pub generated_at_unix: u64,
    /// SHA-256 of the deterministic generator input set; absent for legacy data.
    pub input_hash: Option<String>,
    pub core_method: String,
    pub vm2_method: String,
    /// How many candidate names the VM2 probe tried.
    pub vm2_universe: usize,
    /// Probes that told us nothing clean: a panic (the name dispatched, so it
    /// is in `vm2`, and a panic on a zero-argument call is itself a bug) or a
    /// `(funcall 'NAME)` the compiler refused (`NAME: compile refused: ...`).
    pub vm2_probe_panics: Vec<String>,
    /// `os-arch` of the enumerating host. Platform-gated registrations (`w32-*`,
    /// `ns-*`, `kqueue-*`) make the registry differ between hosts.
    pub generated_on: String,
    /// `core:NAME` / `vm2:NAME` the source registers that that runtime did not
    /// install when enumerated (platform- or feature-gated registrations, dead
    /// registrars). Later drift is measured against this, per runtime, so
    /// these do not read as staleness.
    pub source_only_at_generation: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    pub core: Vec<CoreSubr>,
    pub vm2: Vec<String>,
    pub provenance: RegistryProvenance,
}

/// What changed between two registries. Empty means the same registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistryDiff {
    pub core_added: Vec<String>,
    pub core_removed: Vec<String>,
    /// `name: old -> new` for arity or alias changes.
    pub core_changed: Vec<String>,
    pub vm2_added: Vec<String>,
    pub vm2_removed: Vec<String>,
}

impl RegistryDiff {
    pub fn is_empty(&self) -> bool {
        self.core_added.is_empty()
            && self.core_removed.is_empty()
            && self.core_changed.is_empty()
            && self.vm2_added.is_empty()
            && self.vm2_removed.is_empty()
    }
}

impl fmt::Display for RegistryDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut part = |label: &str, v: &[String]| -> fmt::Result {
            if !v.is_empty() {
                writeln!(f, "  {label} ({}): {}", v.len(), v.join(", "))?;
            }
            Ok(())
        };
        part("core registered, snapshot lacks", &self.core_added)?;
        part("snapshot has, core no longer registers", &self.core_removed)?;
        part("core changed", &self.core_changed)?;
        part("VM2 registered, snapshot lacks", &self.vm2_added)?;
        part("snapshot has, VM2 no longer registers", &self.vm2_removed)
    }
}

impl Registry {
    pub fn core_names(&self) -> BTreeSet<&str> {
        self.core.iter().map(|c| c.name.as_str()).collect()
    }
    pub fn vm2_names(&self) -> BTreeSet<&str> {
        self.vm2.iter().map(String::as_str).collect()
    }

    /// `self` is the old registry (the snapshot), `live` the new one.
    pub fn diff(&self, live: &Registry) -> RegistryDiff {
        let old: std::collections::BTreeMap<&str, &CoreSubr> =
            self.core.iter().map(|c| (c.name.as_str(), c)).collect();
        let new: std::collections::BTreeMap<&str, &CoreSubr> =
            live.core.iter().map(|c| (c.name.as_str(), c)).collect();
        let mut d = RegistryDiff::default();
        for (n, c) in &new {
            match old.get(n) {
                None => d.core_added.push(n.to_string()),
                Some(o) if o != c => d.core_changed.push(format!(
                    "{n}: {} {} -> {} {}",
                    o.subr_name, o.arity, c.subr_name, c.arity
                )),
                _ => {}
            }
        }
        d.core_removed = old
            .keys()
            .filter(|n| !new.contains_key(*n))
            .map(|n| n.to_string())
            .collect();
        let (ov, nv) = (self.vm2_names(), live.vm2_names());
        d.vm2_added = nv.difference(&ov).map(|s| s.to_string()).collect();
        d.vm2_removed = ov.difference(&nv).map(|s| s.to_string()).collect();
        d
    }

    pub fn to_json(&self) -> Json {
        let mut core = self.core.clone();
        core.sort();
        let mut vm2 = self.vm2.clone();
        vm2.sort();
        vm2.dedup();
        let p = &self.provenance;
        json!({
            "schema": 2,
            "note": "Written by `cargo run -p romacs-introspect-live -- --write` from booted runtimes. Do not edit; regenerate. The gate test `snapshot_matches_live_registry` fails when this differs from what the runtimes register.",
            "provenance": {
                "generated_by": p.generated_by,
                "generated_on": p.generated_on,
                "input_hash": p.input_hash,
                "vm2_universe": p.vm2_universe,
                "vm2_probe_panics": p.vm2_probe_panics,
                "source_only_at_generation": p.source_only_at_generation,
            },
            "core": {
                "method": p.core_method,
                "count": core.len(),
                "entries": core.iter().map(|c| {
                    let mut e = json!({"name": c.name, "min": c.arity.min, "max": c.arity.max_json()});
                    if c.subr_name != c.name {
                        e["subr_name"] = json!(c.subr_name);
                    }
                    e
                }).collect::<Vec<_>>(),
            },
            "vm2": {"method": p.vm2_method, "count": vm2.len(), "entries": vm2},
        })
    }

    pub fn from_json(j: &Json) -> Result<Registry, String> {
        let mut core = Vec::new();
        for e in j["core"]["entries"]
            .as_array()
            .ok_or("registry: core.entries missing")?
        {
            let name = e["name"]
                .as_str()
                .ok_or("registry: core entry without name")?
                .to_string();
            let min = e["min"]
                .as_u64()
                .ok_or_else(|| format!("registry: {name}: min"))? as u16;
            let max = match &e["max"] {
                Json::Number(n) => MaxArity::Fixed(n.as_u64().unwrap_or(0) as u16),
                Json::String(s) => {
                    Arity::parse_max(s).ok_or_else(|| format!("registry: {name}: max {s}"))?
                }
                _ => return Err(format!("registry: {name}: max missing")),
            };
            core.push(CoreSubr {
                subr_name: e["subr_name"].as_str().unwrap_or(&name).to_string(),
                name,
                arity: Arity { min, max },
            });
        }
        let vm2 = j["vm2"]["entries"]
            .as_array()
            .ok_or("registry: vm2.entries missing")?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let p = &j["provenance"];
        let strings = |v: &Json| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        };
        Ok(Registry {
            core,
            vm2,
            provenance: RegistryProvenance {
                generated_by: p["generated_by"].as_str().unwrap_or("unknown").to_string(),
                generated_at_unix: p["generated_at_unix"].as_u64().unwrap_or(0),
                generated_on: p["generated_on"].as_str().unwrap_or("unknown").to_string(),
                input_hash: if j["schema"] == 2 {
                    p["input_hash"]
                        .as_str()
                        .filter(|h| {
                            h.len() == 64
                                && h.bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        })
                        .map(String::from)
                } else {
                    None
                },
                core_method: j["core"]["method"].as_str().unwrap_or("").to_string(),
                vm2_method: j["vm2"]["method"].as_str().unwrap_or("").to_string(),
                vm2_universe: p["vm2_universe"].as_u64().unwrap_or(0) as usize,
                vm2_probe_panics: strings(&p["vm2_probe_panics"]),
                source_only_at_generation: strings(&p["source_only_at_generation"]),
            },
        })
    }
}
