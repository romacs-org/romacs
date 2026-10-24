//! romacs introspection: where every primitive is implemented, how it differs
//! from GNU, and how sure we are — derived, never written down.
//!
//! R99: *every answer is derived from the artefact it describes, carries its
//! provenance, and states when it was computed.* So:
//!
//! * **What is registered** comes from the runtimes themselves: a booted
//!   `romacs-core` `Context` enumerated with `mapatoms`, and a fresh VM2 probed
//!   name by name (`romacs-introspect-live`). In-process callers pass that live
//!   registry in ([`Index::with_live_registry`]); the standalone tool uses the
//!   committed snapshot, which the gate checks against the live runtimes.
//! * **Where** — registration sites, implementing functions, divergence rows,
//!   test mentions — is derived by `build.rs` from the tree being compiled, so
//!   it is exactly as new as this binary.
//! * **Absence is never asserted.** A name the index cannot place yields
//!   [`Lookup::NotFound`], which carries what was searched, the coverage of
//!   that search and the likely causes. There is no accessor that turns it into
//!   `None`.
//!
//! No `Value` from any runtime crosses into this crate: everything here is
//! owned strings and integers, so nothing can dangle across a moving collector.

pub mod fingerprint;
pub mod registry;

pub use registry::{Arity, CoreSubr, MaxArity, Registry, RegistryDiff, RegistryProvenance};

use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::OnceLock;

static EMBEDDED_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/index.json"));

// ------------------------------------------------------------------ types

/// Which runtime(s) register a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Implementation {
    Vm2Only,
    CoreOnly,
    Both,
}

impl Implementation {
    pub fn as_str(self) -> &'static str {
        match self {
            Implementation::Vm2Only => "vm2",
            Implementation::CoreOnly => "core",
            Implementation::Both => "vm2+core",
        }
    }
}

/// `crate/file.rs:line`, as derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub file: String,
    pub line: usize,
}

impl Span {
    pub fn parse(s: &str) -> Option<Span> {
        let (f, l) = s.rsplit_once(':')?;
        Some(Span {
            file: f.to_string(),
            line: l.parse().ok()?,
        })
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// The Rust behind one core registration.
#[derive(Clone, Debug)]
pub struct CoreImpl {
    /// Registration call, e.g. `crates/romacs-core/src/.../subrs/mod.rs:3111`.
    pub site: Span,
    /// `SubrSpec::new`, `SubrSpec::fixed4`, `EvaluatorSubr::special`, ...
    pub shape: String,
    /// The implementing path as written at the site, if the shape carries one.
    pub path: Option<String>,
    /// `fn`, `closure`, `evaluator-handler` or `unparsed`.
    pub kind: String,
    /// The defining `fn` (or dispatching match arm), when it could be chosen.
    pub span: Option<Span>,
    /// `unique`, `same-file`, `module-path`, `first-match-arm`, `ambiguous`,
    /// `unresolved` or `unparsed`. Anything but the first two is a guess or a
    /// gap and is labelled so in every rendering.
    pub resolution: String,
    /// Every `fn` of that name, when more than one exists.
    pub candidates: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Vm2Impl {
    /// Entry in VM2's `PRIMITIVES` table, the direct slot assignment, or the
    /// embedded Lisp definition.
    pub site: Span,
    /// `primitive-table`, `defsubr`, `direct-slot` or `lisp-definition`.
    pub kind: String,
    /// `Primitive::Maphash`; empty for a Lisp definition.
    pub variant: String,
    /// First match arm on the variant: where it is dispatched. A heuristic,
    /// labelled so; it may be an arity table rather than the body.
    pub handler: Option<Span>,
}

#[derive(Clone, Debug)]
pub struct Divergence {
    pub id: Option<String>,
    pub ledger: Span,
    pub section: Option<String>,
    /// Can a package observe it (the ledger's own classification).
    pub observable: bool,
    pub planned: bool,
    pub mechanism: String,
    pub gnu: String,
    pub romacs: String,
    pub why: String,
}

/// Which test files name the symbol. A lexical mention, not execution
/// coverage: zero mentions means no scanned test names it directly; a mention
/// does not prove the call ran.
#[derive(Clone, Debug, Default)]
pub struct TestMentions {
    pub files: usize,
    pub examples: Vec<String>,
}

/// Where the index came from and how far to trust it. Attached to every answer.
#[derive(Clone, Debug)]
pub struct Provenance {
    pub build_input_hash: Option<String>,
    /// Commit the spans, ledger rows and test mentions were derived from.
    pub build_commit: Option<String>,
    /// Were the derivation's inputs clean in that tree.
    pub build_inputs_clean: Option<bool>,
    pub build_dirty_inputs: Vec<String>,
    pub derived_at_unix: u64,
    pub git: String,
    pub registry: RegistryProvenance,
    pub freshness: Freshness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// The registry was enumerated from runtimes in this process.
    Live { enumerated_at_unix: u64 },
    /// The committed snapshot, checked against this build's source.
    Snapshot(SnapshotVerdict),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotVerdict {
    /// Matches this build's input hash and platform, and the source shows no
    /// registration the snapshot lacks.
    Current,
    /// Not provably taken from this input hash and platform.
    /// This build's source registers nothing the
    /// snapshot lacks; the gate's live check is what verifies the rest.
    NoDriftDetected,
    /// This build's source registers names the snapshot does not have: the
    /// snapshot is older than the code. Answers about these names are refused.
    Stale { source_registers: Vec<String> },
}

impl Freshness {
    pub fn is_stale(&self) -> bool {
        matches!(self, Freshness::Snapshot(SnapshotVerdict::Stale { .. }))
    }
}

/// What a search covered. Every negative answer carries one.
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub core_registered: usize,
    pub vm2_registered: usize,
    pub vm2_probe_universe: usize,
    pub core_with_site: usize,
    pub core_with_fn_span: usize,
    pub core_span_guessed: usize,
    pub core_span_ambiguous: Vec<String>,
    pub core_span_unresolved: Vec<String>,
    /// Registered in core with no recognised registration site: a known gap.
    pub core_without_site: Vec<String>,
    pub vm2_with_site: usize,
    pub vm2_without_site: Vec<String>,
    /// Registered by source but not by the runtime when the snapshot was taken.
    pub source_only: Vec<String>,
    pub test_files_scanned: usize,
    pub lisp_definitions: usize,
}

#[derive(Clone, Debug)]
pub struct Described {
    pub name: String,
    pub implementation: Implementation,
    /// `subr-arity` from core. VM2 declares no arity table; its handlers check
    /// argument counts themselves, so a VM2-only name has `None` here.
    pub arity: Option<Arity>,
    /// `Some(primary)` when this symbol is an alias of another subr.
    pub alias_of: Option<String>,
    pub special_form: bool,
    pub core: Vec<CoreImpl>,
    pub vm2: Vec<Vm2Impl>,
    pub divergences: Vec<Divergence>,
    pub tests: TestMentions,
    /// Also defined in Lisp (a Lisp definition may shadow the primitive).
    pub lisp_definition: Option<(Span, String)>,
    /// Gaps and guesses in this answer, in words.
    pub caveats: Vec<String>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Defined in Lisp, so not a primitive.
    LispDefined { at: Span, form: String },
    /// Source registers it but the registry does not: a stale snapshot, a
    /// feature-gated registration, or a registrar that never runs.
    RegisteredInSourceOnly { site: Span },
    /// The name is quoted in runtime source but in no recognised registration.
    QuotedInSource { at: Span },
    /// Registered names that differ by a typo.
    NearMiss { names: Vec<String> },
    /// Nothing scanned mentions it.
    NoTrace,
}

/// A refusal to assert absence. Says what was searched and what to suspect.
#[derive(Clone, Debug)]
pub struct Absence {
    pub name: String,
    pub searched: String,
    pub coverage: Coverage,
    pub likely_causes: Vec<Cause>,
    pub provenance: Provenance,
}

/// The answer to "what is NAME". There is deliberately no `Option` form: a
/// caller must handle [`Lookup::NotFound`], which is not a statement that the
/// name does not exist.
#[must_use]
#[derive(Clone, Debug)]
pub enum Lookup {
    Found(Box<Described>),
    NotFound(Box<Absence>),
}

/// One filter of a set query; several are intersected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetQuery {
    All,
    Implementation(Implementation),
    /// Has at least one divergence row; `observable_only` keeps rows the
    /// ledger classifies as observable by a package.
    Divergent {
        observable_only: bool,
    },
    /// No scanned test file mentions it.
    NoTestMentions,
    /// No resolved implementing `fn` span.
    NoSourceSpan,
    SpecialForms,
}

#[must_use]
#[derive(Clone, Debug)]
pub struct SetAnswer {
    pub query: Vec<SetQuery>,
    pub names: Vec<String>,
    /// Size of the population the filters ran over.
    pub considered: usize,
    pub caveats: Vec<String>,
    pub provenance: Provenance,
}

/// Lisp names implemented by a Rust function, VM2 variant or span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Implements {
    pub lisp_name: String,
    pub runtime: &'static str,
    pub matched: String,
}

#[must_use]
#[derive(Clone, Debug)]
pub enum Reverse {
    Found {
        query: String,
        names: Vec<Implements>,
        provenance: Provenance,
    },
    NotFound {
        query: String,
        searched: String,
        maybe_among: Vec<String>,
        coverage: Box<Coverage>,
        provenance: Provenance,
    },
}

// ------------------------------------------------------------------ index

pub struct Index {
    registry: Registry,
    src: &'static Json,
    provenance: Provenance,
    core: BTreeMap<String, CoreSubr>,
    vm2: BTreeSet<String>,
    drift: Vec<String>,
}

fn embedded_json() -> &'static Json {
    static J: OnceLock<Json> = OnceLock::new();
    J.get_or_init(|| {
        serde_json::from_str(EMBEDDED_JSON).expect("romacs-introspect: embedded index is not JSON")
    })
}

fn strings(v: &Json) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(String::from))
        .collect()
}

/// Resolutions that read a definition directly rather than choosing one.
const DIRECT: [&str; 2] = ["unique", "same-file"];

impl Index {
    /// The index built into this binary, over the committed registry snapshot.
    pub fn embedded() -> &'static Index {
        static I: OnceLock<Index> = OnceLock::new();
        I.get_or_init(|| {
            let reg = Registry::from_json(&embedded_json()["registry"])
                .expect("romacs-introspect: embedded registry");
            Index::new(reg, None)
        })
    }

    /// An index over a registry enumerated in this process. Spans and ledger
    /// rows are still this build's; freshness is `Live`.
    pub fn with_live_registry(registry: Registry) -> Index {
        let at = registry.provenance.generated_at_unix;
        Index::new(registry, Some(at))
    }

    /// An index over an arbitrary snapshot, judged against this build's
    /// source. Used to check what a given snapshot would claim.
    pub fn with_snapshot(registry: Registry) -> Index {
        Index::new(registry, None)
    }

    fn new(registry: Registry, live_at: Option<u64>) -> Index {
        let src = embedded_json();
        let core: BTreeMap<String, CoreSubr> = registry
            .core
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let vm2: BTreeSet<String> = registry.vm2.iter().cloned().collect();
        let expected: BTreeSet<&str> = registry
            .provenance
            .source_only_at_generation
            .iter()
            .map(String::as_str)
            .collect();
        // Per runtime, not over the union: VM2 adding a name core already has
        // leaves the union unchanged while turning `core` into `vm2+core`.
        let mut drift: Vec<String> = strings(&src["source_registered"]["core"])
            .into_iter()
            .filter(|n| !core.contains_key(n) && !expected.contains(format!("core:{n}").as_str()))
            .chain(
                strings(&src["source_registered"]["vm2"])
                    .into_iter()
                    .filter(|n| {
                        !vm2.contains(n) && !expected.contains(format!("vm2:{n}").as_str())
                    }),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let b = &src["build"];
        let build_commit = b["commit"].as_str().map(String::from);
        let build_clean = b["inputs_clean"].as_bool();
        let freshness = match live_at {
            Some(at) => {
                // A live registry cannot be stale. Source registrations the
                // runtime did not install are reported by coverage instead.
                drift.clear();
                Freshness::Live {
                    enumerated_at_unix: at,
                }
            }
            None if !drift.is_empty() => Freshness::Snapshot(SnapshotVerdict::Stale {
                source_registers: drift.clone(),
            }),
            None if registry.provenance.input_hash.is_some()
                && registry.provenance.input_hash.as_deref() == b["input_hash"].as_str()
                && Some(registry.provenance.generated_on.as_str())
                    == b["generated_on"].as_str() =>
            {
                Freshness::Snapshot(SnapshotVerdict::Current)
            }
            None => Freshness::Snapshot(SnapshotVerdict::NoDriftDetected),
        };
        let provenance = Provenance {
            build_input_hash: b["input_hash"].as_str().map(String::from),
            build_commit,
            build_inputs_clean: build_clean,
            build_dirty_inputs: strings(&b["dirty_inputs"]),
            derived_at_unix: b["derived_at_unix"].as_u64().unwrap_or(0),
            git: b["git"].as_str().unwrap_or("unknown").to_string(),
            registry: registry.provenance.clone(),
            freshness,
        };
        Index {
            registry,
            src,
            provenance,
            core,
            vm2,
            drift,
        }
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Every name registered by either runtime.
    pub fn names(&self) -> BTreeSet<&str> {
        self.core
            .keys()
            .map(String::as_str)
            .chain(self.vm2.iter().map(String::as_str))
            .collect()
    }

    fn implementation_of(&self, name: &str) -> Option<Implementation> {
        match (self.vm2.contains(name), self.core.contains_key(name)) {
            (true, true) => Some(Implementation::Both),
            (true, false) => Some(Implementation::Vm2Only),
            (false, true) => Some(Implementation::CoreOnly),
            (false, false) => None,
        }
    }

    fn core_impls(&self, name: &str) -> Vec<CoreImpl> {
        self.src["core_sites"][name]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let i = &s["implementation"];
                CoreImpl {
                    site: Span::parse(s["site"].as_str().unwrap_or("")).unwrap_or(Span {
                        file: String::new(),
                        line: 0,
                    }),
                    shape: s["shape"].as_str().unwrap_or("").to_string(),
                    path: i["path"]
                        .as_str()
                        .filter(|p| !p.is_empty())
                        .map(String::from),
                    kind: i["kind"].as_str().unwrap_or("").to_string(),
                    span: i["span"].as_str().and_then(Span::parse),
                    resolution: i["resolution"].as_str().unwrap_or("").to_string(),
                    candidates: strings(&i["candidates"]),
                }
            })
            .collect()
    }

    fn vm2_impls(&self, name: &str) -> Vec<Vm2Impl> {
        self.src["vm2_sites"][name]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| Vm2Impl {
                site: Span::parse(s["site"].as_str().unwrap_or("")).unwrap_or(Span {
                    file: String::new(),
                    line: 0,
                }),
                kind: s["kind"].as_str().unwrap_or("").to_string(),
                variant: s["variant"].as_str().unwrap_or("").to_string(),
                handler: s["handler"].as_str().and_then(Span::parse),
            })
            .collect()
    }

    fn divergences(&self, name: &str) -> Vec<Divergence> {
        let rows = &self.src["divergences"]["rows"];
        self.src["divergences"]["by_symbol"][name]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i.as_u64())
            .map(|i| {
                let r = &rows[i as usize];
                let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
                Divergence {
                    id: r["id"].as_str().map(String::from),
                    ledger: Span {
                        file: s("source"),
                        line: r["line"].as_u64().unwrap_or(0) as usize,
                    },
                    section: r["section"].as_str().map(String::from),
                    observable: r["observable"].as_bool().unwrap_or(false),
                    planned: r["planned"].as_bool().unwrap_or(false),
                    mechanism: s("mechanism"),
                    gnu: s("gnu"),
                    romacs: s("romacs"),
                    why: s("why"),
                }
            })
            .collect()
    }

    fn tests(&self, name: &str) -> TestMentions {
        let t = &self.src["tests"]["mentions"][name];
        TestMentions {
            files: t["files"].as_u64().unwrap_or(0) as usize,
            examples: strings(&t["examples"]),
        }
    }

    fn lisp_definition(&self, name: &str) -> Option<(Span, String)> {
        let d = self.src["lisp"]["defs"][name].as_array()?;
        let file = self.src["lisp"]["files"][d.first()?.as_u64()? as usize].as_str()?;
        Some((
            Span {
                file: file.to_string(),
                line: d.get(1)?.as_u64()? as usize,
            },
            d.get(2)?.as_str()?.to_string(),
        ))
    }

    fn test_files_scanned(&self) -> usize {
        self.src["tests"]["universe"]["files"].as_u64().unwrap_or(0) as usize
    }

    pub fn coverage(&self) -> Coverage {
        let mut c = Coverage {
            core_registered: self.core.len(),
            vm2_registered: self.vm2.len(),
            vm2_probe_universe: self.registry.provenance.vm2_universe,
            source_only: self.registry.provenance.source_only_at_generation.clone(),
            test_files_scanned: self.test_files_scanned(),
            lisp_definitions: self.src["lisp"]["defs"].as_object().map_or(0, |m| m.len()),
            ..Coverage::default()
        };
        for name in self.core.keys() {
            let impls = self.core_impls(name);
            if impls.is_empty() {
                c.core_without_site.push(name.clone());
                continue;
            }
            c.core_with_site += 1;
            if impls.iter().any(|i| i.span.is_some()) {
                c.core_with_fn_span += 1;
                if !impls
                    .iter()
                    .any(|i| DIRECT.contains(&i.resolution.as_str()))
                {
                    c.core_span_guessed += 1;
                }
            } else if impls.iter().any(|i| i.resolution == "ambiguous") {
                c.core_span_ambiguous.push(name.clone());
            } else {
                c.core_span_unresolved.push(name.clone());
            }
        }
        for name in &self.vm2 {
            if self.src["vm2_sites"][name.as_str()].is_array() {
                c.vm2_with_site += 1;
            } else {
                c.vm2_without_site.push(name.clone());
            }
        }
        c
    }

    fn searched(&self) -> String {
        let p = &self.registry.provenance;
        let kind = match self.provenance.freshness {
            Freshness::Live { .. } => "live",
            Freshness::Snapshot(_) => "snapshot",
        };
        format!(
            "the {kind} registry: {} romacs-core subrs (every subr in a fresh Context, via mapatoms) and {} VM2 \
             functions (a fresh VM probed with {} candidate names); plus, for leads, this build's Lisp \
             definitions under lisp/, runtime registration sites, and the divergence ledgers",
            self.core.len(),
            self.vm2.len(),
            p.vm2_universe
        )
    }

    /// Everything known about NAME, or a reasoned refusal.
    pub fn describe(&self, name: &str) -> Lookup {
        // A drifted name is refused even when one runtime has it: the snapshot
        // is older than the code for exactly this name, so its implementation
        // answer is the confident-and-wrong kind this index exists to prevent.
        if self.drift.iter().any(|d| d == name) {
            return Lookup::NotFound(Box::new(self.absence(name)));
        }
        let Some(implementation) = self.implementation_of(name) else {
            return Lookup::NotFound(Box::new(self.absence(name)));
        };
        let core = if self.core.contains_key(name) {
            self.core_impls(name)
        } else {
            vec![]
        };
        let vm2 = if self.vm2.contains(name) {
            self.vm2_impls(name)
        } else {
            vec![]
        };
        let subr = self.core.get(name);
        let mut caveats = Vec::new();
        if subr.is_some() && core.is_empty() {
            let lead = self.src["leads"][name].as_str().unwrap_or("none");
            caveats.push(format!(
                "registered in core, but no recognised registration shape names it: the derivation has a gap \
                 here (first literal occurrence: {lead})"
            ));
        }
        if self.vm2.contains(name) && vm2.is_empty() {
            caveats.push("bound in VM2, but no recognised VM2 registration site names it".into());
        }
        for c in &core {
            match c.resolution.as_str() {
                "unique" | "same-file" => {}
                "module-path" => caveats.push(format!(
                    "{}: several fns share this name; the span was chosen by matching the path's modules to file \
                     paths, not by the compiler (candidates: {})",
                    c.path.as_deref().unwrap_or("?"),
                    c.candidates.join(", ")
                )),
                "first-match-arm" => caveats.push(format!(
                    "{}: span is the first match arm naming the handler, not a resolved fn",
                    c.path.as_deref().unwrap_or("?")
                )),
                "ambiguous" => caveats.push(format!(
                    "{}: ambiguous, {} fns share this name and none was chosen: {}",
                    c.path.as_deref().unwrap_or("?"),
                    c.candidates.len(),
                    c.candidates.join(", ")
                )),
                other => caveats.push(format!(
                    "implementation at {} is {other}: the registration does not name a fn the derivation can follow",
                    c.site
                )),
            }
        }
        if vm2.iter().any(|v| v.handler.is_some()) {
            caveats.push("VM2 handler spans are the first match arm on the variant: a heuristic, which may be an arity table rather than the body".into());
        }
        if subr.is_none() {
            caveats.push("VM2 declares no arity table; its handlers check argument counts themselves, so no arity is reported".into());
        }
        let lisp_definition = self.lisp_definition(name);
        if let Some((at, form)) = &lisp_definition {
            caveats.push(format!(
                "also defined in Lisp ({form} at {at}): once that file loads, the Lisp definition may replace this primitive"
            ));
        }
        caveats.extend(self.freshness_caveat());
        Lookup::Found(Box::new(Described {
            name: name.to_string(),
            implementation,
            arity: subr.map(|s| s.arity),
            alias_of: subr
                .filter(|s| s.subr_name != s.name)
                .map(|s| s.subr_name.clone()),
            special_form: subr.is_some_and(|s| s.arity.max == MaxArity::Unevalled),
            core,
            vm2,
            divergences: self.divergences(name),
            tests: self.tests(name),
            lisp_definition,
            caveats,
            provenance: self.provenance.clone(),
        }))
    }

    fn freshness_caveat(&self) -> Option<String> {
        match &self.provenance.freshness {
            Freshness::Live { .. } | Freshness::Snapshot(SnapshotVerdict::Current) => None,
            Freshness::Snapshot(SnapshotVerdict::NoDriftDetected) => Some(format!(
                "registry input hash/platform does not match this build: snapshot {} ({}), build {}. No source drift detected; run the live snapshot check",
                self.registry
                    .provenance
                    .input_hash
                    .as_deref()
                    .unwrap_or("legacy/unknown"),
                self.registry.provenance.generated_on,
                self.provenance
                    .build_input_hash
                    .as_deref()
                    .unwrap_or("unknown")
            )),
            Freshness::Snapshot(SnapshotVerdict::Stale { source_registers }) => Some(format!(
                "STALE: the registry snapshot is older than this build's source, which registers {} name(s) the \
                 snapshot lacks ({}). Regenerate: cargo run -p romacs-introspect-live -- --write",
                source_registers.len(),
                source_registers
                    .iter()
                    .take(10)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn absence(&self, name: &str) -> Absence {
        let mut causes = Vec::new();
        for (key, rt) in [("core_sites", "core"), ("vm2_sites", "vm2")] {
            if let Some(site) = self.src[key][name].as_array().and_then(|a| a.first()) {
                let _ = rt;
                if let Some(s) = site["site"].as_str().and_then(Span::parse) {
                    causes.push(Cause::RegisteredInSourceOnly { site: s });
                }
            }
        }
        if let Some(lead) = self.src["leads"][name].as_str().and_then(Span::parse) {
            causes.push(Cause::QuotedInSource { at: lead });
        }
        if let Some((at, form)) = self.lisp_definition(name) {
            causes.push(Cause::LispDefined { at, form });
        }
        let near = self.near_misses(name);
        if !near.is_empty() {
            causes.push(Cause::NearMiss { names: near });
        }
        if causes.is_empty() {
            causes.push(Cause::NoTrace);
        }
        Absence {
            name: name.to_string(),
            searched: self.searched(),
            coverage: self.coverage(),
            likely_causes: causes,
            provenance: self.provenance.clone(),
        }
    }

    fn near_misses(&self, name: &str) -> Vec<String> {
        if name.chars().count() < 3 {
            return vec![];
        }
        let mut scored: Vec<(usize, &str)> = self
            .names()
            .into_iter()
            .filter_map(|n| {
                let d = edit_distance(name, n);
                (d <= 2).then_some((d, n))
            })
            .collect();
        scored.sort();
        scored
            .into_iter()
            .take(5)
            .map(|(_, n)| n.to_string())
            .collect()
    }

    pub fn query(&self, filters: &[SetQuery]) -> SetAnswer {
        let population: Vec<&str> = self.names().into_iter().collect();
        let mut caveats = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for &n in &population {
            let keep = filters.iter().all(|f| match f {
                SetQuery::All => true,
                SetQuery::Implementation(i) => self.implementation_of(n) == Some(*i),
                SetQuery::Divergent { observable_only } => self
                    .divergences(n)
                    .iter()
                    .any(|d| d.observable || !observable_only),
                SetQuery::NoTestMentions => self.tests(n).files == 0,
                SetQuery::NoSourceSpan => {
                    (self.core.contains_key(n)
                        && !self.core_impls(n).iter().any(|i| i.span.is_some()))
                        || (self.vm2.contains(n)
                            && !self.core.contains_key(n)
                            && self.vm2_impls(n).is_empty())
                }
                SetQuery::SpecialForms => self
                    .core
                    .get(n)
                    .is_some_and(|s| s.arity.max == MaxArity::Unevalled),
            });
            if keep {
                names.push(n.to_string());
            }
        }
        for f in filters {
            match f {
                SetQuery::Divergent { .. } => {
                    let other = self.non_primitive_divergence_symbols();
                    caveats.push(format!(
                        "{} divergence-ledger symbols are not registered primitives and are outside this set: {}",
                        other.len(),
                        other.join(", ")
                    ));
                }
                SetQuery::NoTestMentions => caveats.push(format!(
                    "test mentions are lexical: `(name` or `#'name` in {} scanned test files. A name listed here \
                     is named by no scanned test; a name not listed may still be untested, and a name reached only \
                     indirectly is listed",
                    self.test_files_scanned()
                )),
                SetQuery::NoSourceSpan => caveats.push(
                    "a span means a resolved implementing fn (or VM2 site); ambiguous, unresolved and closure \
                     registrations are listed here, not hidden"
                        .into(),
                ),
                SetQuery::Implementation(Implementation::Vm2Only | Implementation::Both) => caveats.push(
                    "VM2's side includes its embedded Lisp definitions (kind lisp-definition) as well as primitives"
                        .into(),
                ),
                _ => {}
            }
        }
        caveats.extend(self.freshness_caveat());
        SetAnswer {
            query: filters.to_vec(),
            names,
            considered: population.len(),
            caveats,
            provenance: self.provenance.clone(),
        }
    }

    /// Given a Rust fn (`builtin_maphash`, `hashtab::builtin_maphash`), a VM2
    /// variant (`Primitive::Maphash`), or a span (`file.rs:909`): the Lisp
    /// names it implements.
    pub fn implements(&self, rust: &str) -> Reverse {
        let q = rust.trim();
        let last = q.rsplit("::").next().unwrap_or(q);
        let mut found = Vec::new();
        let mut maybe = Vec::new();
        let is_span = Span::parse(q).is_some_and(|s| s.file.ends_with(".rs"));
        for name in self.core.keys() {
            for i in self.core_impls(name) {
                let hit = if is_span {
                    i.span
                        .as_ref()
                        .is_some_and(|s| s.to_string() == q)
                        .then(|| "implementing fn span".to_string())
                        .or_else(|| {
                            (i.site.to_string() == q).then(|| "registration site".to_string())
                        })
                } else if let Some(p) = &i.path {
                    let plast = p.rsplit("::").next().unwrap_or(p);
                    let module_ok = !q.contains("::")
                        || p.ends_with(q)
                        || q.ends_with(p.as_str())
                        || q.rsplit("::").skip(1).all(|m| {
                            i.span
                                .as_ref()
                                .is_some_and(|s| s.file.split(['/', '.']).any(|c| c == m))
                                || matches!(m, "crate" | "super" | "self" | "emacs_core")
                        });
                    (plast == last && module_ok).then(|| format!("{} `{p}`", i.kind))
                } else {
                    None
                };
                match hit {
                    Some(how) => found.push(Implements {
                        lisp_name: name.clone(),
                        runtime: "core",
                        matched: how,
                    }),
                    None => {
                        if !DIRECT.contains(&i.resolution.as_str())
                            && (i.path.is_none()
                                || i.candidates.iter().any(|c| c.contains(last))
                                || i.path.as_deref().is_some_and(|p| p.ends_with(last)))
                        {
                            maybe.push(name.clone());
                        }
                    }
                }
            }
        }
        for name in &self.vm2 {
            for v in self.vm2_impls(name) {
                let hit = if is_span {
                    (v.site.to_string() == q
                        || v.handler.as_ref().is_some_and(|h| h.to_string() == q))
                    .then(|| "VM2 site or handler span".to_string())
                } else {
                    (!v.variant.is_empty()
                        && (v.variant == q || v.variant.ends_with(&format!("::{q}"))))
                    .then(|| format!("VM2 variant `{}`", v.variant))
                };
                if let Some(how) = hit {
                    found.push(Implements {
                        lisp_name: name.clone(),
                        runtime: "vm2",
                        matched: how,
                    });
                }
            }
        }
        if found.is_empty() {
            maybe.sort();
            maybe.dedup();
            return Reverse::NotFound {
                query: q.to_string(),
                searched: format!(
                    "the implementing paths and spans of {} core registrations and the variants and sites of {} VM2 \
                     functions in this build",
                    self.core.len(),
                    self.vm2.len()
                ),
                maybe_among: maybe,
                coverage: Box::new(self.coverage()),
                provenance: self.provenance.clone(),
            };
        }
        Reverse::Found {
            query: q.to_string(),
            names: found,
            provenance: self.provenance.clone(),
        }
    }

    /// Names the source registers that are absent from the registry, beyond
    /// those already absent when the snapshot was taken.
    pub fn drift(&self) -> Vec<String> {
        self.drift.clone()
    }

    /// Divergence-ledger symbols that are not registered primitives.
    pub fn non_primitive_divergence_symbols(&self) -> Vec<String> {
        self.src["divergences"]["by_symbol"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
            .filter(|k| self.implementation_of(k).is_none())
            .cloned()
            .collect()
    }

    /// Names the source registers (core, VM2): the independent side of the
    /// cross-check against the registry.
    pub fn source_registered(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        (
            strings(&self.src["source_registered"]["core"])
                .into_iter()
                .collect(),
            strings(&self.src["source_registered"]["vm2"])
                .into_iter()
                .collect(),
        )
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

// -------------------------------------------------------------- rendering
//
// R100 item 8: one structured answer, the human text rendered FROM it, so the
// two cannot disagree.

impl Provenance {
    pub fn to_json(&self) -> Json {
        let r = &self.registry;
        json!({
            "build": {
                "input_hash": self.build_input_hash,
                "commit": self.build_commit,
                "inputs_clean": self.build_inputs_clean,
                "dirty_inputs": self.build_dirty_inputs,
                "derived_at_unix": self.derived_at_unix,
                "git": self.git,
                "derived_by": "crates/romacs-introspect/build.rs at compile time",
            },
            "registry": {
                "generated_by": r.generated_by,
                "generated_on": r.generated_on,
                "input_hash": r.input_hash,
                "core_method": r.core_method,
                "vm2_method": r.vm2_method,
            },
            "derived_at_unix": self.derived_at_unix,
            "freshness": match &self.freshness {
                Freshness::Live { enumerated_at_unix } => json!({"state": "live", "enumerated_at_unix": enumerated_at_unix}),
                Freshness::Snapshot(SnapshotVerdict::Current) => json!({"state": "current"}),
                Freshness::Snapshot(SnapshotVerdict::NoDriftDetected) => json!({"state": "no-drift-detected",
                    "meaning": "snapshot not provably taken from this tree (different input hash or platform); this build's source registers nothing it lacks; the gate's live check verifies the rest"}),
                Freshness::Snapshot(SnapshotVerdict::Stale { source_registers }) => json!({"state": "stale",
                    "source_registers": source_registers,
                    "fix": "cargo run -p romacs-introspect-live -- --write"}),
            },
        })
    }
}

impl Coverage {
    pub fn to_json(&self) -> Json {
        json!({
            "core_registered": self.core_registered,
            "core_with_site": self.core_with_site,
            "core_without_site": self.core_without_site,
            "core_with_fn_span": self.core_with_fn_span,
            "core_span_guessed": self.core_span_guessed,
            "core_span_ambiguous": self.core_span_ambiguous,
            "core_span_unresolved": self.core_span_unresolved,
            "vm2_registered": self.vm2_registered,
            "vm2_probe_universe": self.vm2_probe_universe,
            "vm2_with_site": self.vm2_with_site,
            "vm2_without_site": self.vm2_without_site,
            "source_registered_but_not_by_the_runtime": self.source_only,
            "test_files_scanned": self.test_files_scanned,
            "lisp_definitions": self.lisp_definitions,
        })
    }
}

impl Cause {
    pub fn to_json(&self) -> Json {
        match self {
            Cause::LispDefined { at, form } => {
                json!({"cause": "lisp-defined", "at": at.to_string(), "form": form,
                "meaning": "defined in Lisp, so it is not a primitive"})
            }
            Cause::RegisteredInSourceOnly { site } => {
                json!({"cause": "registered-in-source-only", "site": site.to_string(),
                "meaning": "the source registers it but the registry does not: a stale snapshot, a feature- or platform-gated registration, or a registrar that never runs"})
            }
            Cause::QuotedInSource { at } => {
                json!({"cause": "quoted-in-source", "at": at.to_string()})
            }
            Cause::NearMiss { names } => json!({"cause": "near-miss", "registered": names}),
            Cause::NoTrace => json!({"cause": "no-trace",
                "meaning": "nothing scanned names it; probably not defined in this tree, but this index does not cover every Lisp form (e.g. names built by macros), so verify"}),
        }
    }
}

impl Lookup {
    /// The structured answer: what an agent reads in one round trip.
    pub fn to_json(&self) -> Json {
        match self {
            Lookup::Found(d) => json!({
                "found": true,
                "name": d.name,
                "implementation": d.implementation.as_str(),
                "arity": d.arity.map(|a| json!({"min": a.min, "max": a.max_json(), "source": "subr-arity in romacs-core"})),
                "alias_of": d.alias_of,
                "special_form": d.special_form,
                "core": d.core.iter().map(|c| json!({
                    "site": c.site.to_string(), "shape": c.shape, "path": c.path, "kind": c.kind,
                    "span": c.span.as_ref().map(Span::to_string), "resolution": c.resolution, "candidates": c.candidates,
                })).collect::<Vec<_>>(),
                "vm2": d.vm2.iter().map(|v| json!({
                    "site": v.site.to_string(), "kind": v.kind, "variant": v.variant,
                    "handler": v.handler.as_ref().map(Span::to_string),
                    "handler_resolution": "first match arm naming the variant (heuristic)",
                })).collect::<Vec<_>>(),
                "divergences": d.divergences.iter().map(|r| json!({
                    "id": r.id, "ledger": r.ledger.to_string(), "section": r.section,
                    "observable_by_a_package": r.observable, "planned": r.planned,
                    "mechanism": r.mechanism, "gnu": r.gnu, "romacs": r.romacs, "why": r.why,
                })).collect::<Vec<_>>(),
                "tests": {"files_mentioning": d.tests.files, "examples": d.tests.examples,
                          "rule": "lexical mention in a scanned test file, not execution coverage"},
                "lisp_definition": d.lisp_definition.as_ref().map(|(at, form)| json!({"at": at.to_string(), "form": form})),
                "caveats": d.caveats,
                "provenance": d.provenance.to_json(),
            }),
            Lookup::NotFound(a) => json!({
                "found": false,
                "name": a.name,
                "statement": format!("{} is not found by this index -- which is not the same as absent. See searched, coverage and likely_causes.", a.name),
                "searched": a.searched,
                "coverage": a.coverage.to_json(),
                "likely_causes": a.likely_causes.iter().map(Cause::to_json).collect::<Vec<_>>(),
                "provenance": a.provenance.to_json(),
            }),
        }
    }
}

impl SetAnswer {
    pub fn to_json(&self) -> Json {
        json!({
            "query": self.query.iter().map(|q| format!("{q:?}")).collect::<Vec<_>>(),
            "count": self.names.len(),
            "considered": self.considered,
            "names": self.names,
            "caveats": self.caveats,
            "provenance": self.provenance.to_json(),
        })
    }
}

impl Reverse {
    pub fn to_json(&self) -> Json {
        match self {
            Reverse::Found {
                query,
                names,
                provenance,
            } => json!({
                "found": true, "query": query,
                "implements": names.iter().map(|i| json!({"lisp": i.lisp_name, "runtime": i.runtime, "matched": i.matched})).collect::<Vec<_>>(),
                "provenance": provenance.to_json(),
            }),
            Reverse::NotFound {
                query,
                searched,
                maybe_among,
                coverage,
                provenance,
            } => json!({
                "found": false, "query": query,
                "statement": format!("no registration in this index resolves to {query} -- which is not the same as implementing nothing."),
                "searched": searched,
                "maybe_among": maybe_among,
                "maybe_among_meaning": "registrations whose implementing fn could not be resolved unambiguously or was not parsed; the query could be behind one of them",
                "coverage": coverage.to_json(),
                "provenance": provenance.to_json(),
            }),
        }
    }
}

/// Render any answer's JSON as indented text. The text has no content the
/// JSON lacks.
pub fn render_text(j: &Json) -> String {
    fn go(out: &mut String, j: &Json, indent: usize, key: Option<&str>) {
        let pad = "  ".repeat(indent);
        let label = key.map(|k| format!("{k}: ")).unwrap_or_default();
        match j {
            Json::Object(m) => {
                if key.is_some() {
                    out.push_str(&format!("{pad}{label}\n"));
                }
                for (k, v) in m {
                    if v.is_null() || v.as_array().is_some_and(Vec::is_empty) {
                        continue;
                    }
                    go(out, v, indent + usize::from(key.is_some()), Some(k));
                }
            }
            Json::Array(a) if a.iter().all(|v| !v.is_object() && !v.is_array()) => {
                let items: Vec<String> = a
                    .iter()
                    .map(|v| v.as_str().map_or(v.to_string(), String::from))
                    .collect();
                out.push_str(&format!("{pad}{label}{}\n", items.join(", ")));
            }
            Json::Array(a) => {
                out.push_str(&format!("{pad}{label}\n"));
                for (i, v) in a.iter().enumerate() {
                    go(out, v, indent + 1, Some(&format!("[{i}]")));
                }
            }
            Json::String(s) => out.push_str(&format!("{pad}{label}{s}\n")),
            other => out.push_str(&format!("{pad}{label}{other}\n")),
        }
    }
    let mut out = String::new();
    go(&mut out, j, 0, None);
    out
}

impl fmt::Display for Lookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Lookup::NotFound(a) = self {
            writeln!(
                f,
                "{}: NOT FOUND by this index -- which is not the same as absent.",
                a.name
            )?;
        }
        f.write_str(&render_text(&self.to_json()))
    }
}
