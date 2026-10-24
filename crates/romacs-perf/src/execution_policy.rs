//! Request-owned runtime policy for controlled same-binary comparisons.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// One explicit setting, or removal of an inherited setting.
/// CLI spelling: NAME=VALUE sets; NAME alone restores the runtime default.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExecutionOverride {
    name: String,
    value: Option<String>,
}

impl FromStr for ExecutionOverride {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (name, value) = match input.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (input, None),
        };
        let supported = matches!(
            name,
            "ROMACS_REGEX_DFA"
                | "ROMACS_REGEX_DFA_COLD"
                | "ROMACS_REGEX_DFA_FIRST_STEP"
                | "ROMACS_REGEX_SUFFIX_LITERAL"
                | "ROMACS_COMPARE_STRINGS_POS_CACHE"
                | "ROMACS_REGEX_SHORT_LITERAL"
                | "ROMACS_EMACS_MULE_PREPARED"
                | "ROMACS_OVERLAY_LOCAL_MOVE"
        );
        if !supported {
            return Err(format!(
                "unsupported execution setting {name:?}; expected a supported ROMACS execution setting"
            ));
        }
        if let Some(value) = value {
            let valid = match name {
                "ROMACS_REGEX_DFA_COLD"
                | "ROMACS_REGEX_DFA_FIRST_STEP"
                | "ROMACS_REGEX_SUFFIX_LITERAL"
                | "ROMACS_COMPARE_STRINGS_POS_CACHE"
                | "ROMACS_REGEX_SHORT_LITERAL" => {
                    matches!(
                        value,
                        "0" | "off" | "false" | "no" | "1" | "on" | "true" | "yes"
                    )
                }
                "ROMACS_REGEX_DFA" => matches!(value, "off" | "on" | "verify"),
                "ROMACS_EMACS_MULE_PREPARED" => {
                    matches!(value, "0" | "off" | "false" | "no" | "1" | "on" | "true")
                }
                "ROMACS_OVERLAY_LOCAL_MOVE" => matches!(value, "off" | "on"),
                _ => unreachable!("validated name"),
            };
            if !valid {
                return Err(format!("invalid value {value:?} for {name}"));
            }
        }
        Ok(Self {
            name: name.to_owned(),
            value: value.map(str::to_owned),
        })
    }
}

impl TryFrom<String> for ExecutionOverride {
    type Error = String;
    fn try_from(input: String) -> Result<Self, Self::Error> {
        input.parse()
    }
}

impl From<ExecutionOverride> for String {
    fn from(setting: ExecutionOverride) -> Self {
        match setting.value {
            Some(value) => format!("{}={value}", setting.name),
            None => setting.name,
        }
    }
}

/// Validated actions, with at most one action per knob.
/// Empty means inherit the request's captured parent environment.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "Vec<ExecutionOverride>", into = "Vec<ExecutionOverride>")]
pub struct ExecutionOverrides(Vec<ExecutionOverride>);

impl TryFrom<Vec<ExecutionOverride>> for ExecutionOverrides {
    type Error = String;
    fn try_from(mut settings: Vec<ExecutionOverride>) -> Result<Self, Self::Error> {
        settings.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(pair) = settings
            .windows(2)
            .find(|pair| pair[0].name == pair[1].name)
        {
            return Err(format!("duplicate execution setting {}", pair[0].name));
        }
        Ok(Self(settings))
    }
}

impl From<ExecutionOverrides> for Vec<ExecutionOverride> {
    fn from(settings: ExecutionOverrides) -> Self {
        settings.0
    }
}

impl ExecutionOverrides {
    /// Merge without changing the process environment or the other arm.
    pub(crate) fn apply_to(&self, environment: &mut BTreeMap<String, OsString>) {
        for setting in &self.0 {
            if let Some(value) = &setting.value {
                environment.insert(setting.name.clone(), OsString::from(value));
            } else {
                environment.remove(&setting.name);
            }
        }
    }

    /// Every requested action must be reflected in the child's provenance.
    pub(crate) fn validate_recorded(
        &self,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        for setting in &self.0 {
            let actual = environment.get(&setting.name);
            if actual != setting.value.as_ref() {
                return Err(format!(
                    "execution setting {}: expected {:?}, recorded {:?}",
                    setting.name, setting.value, actual
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "execution_policy/tests/execution_policy_test.rs"]
mod tests;

#[cfg(test)]
#[path = "execution_policy/tests/gnu_sort_capture.rs"]
mod gnu_sort_capture;
