use super::*;

fn policy(settings: &[&str]) -> ExecutionOverrides {
    settings
        .iter()
        .map(|s| s.parse().unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

#[test]
fn arm_actions_do_not_mutate_inheritance_or_the_other_arm() {
    let inherited = BTreeMap::from([
        ("ROMACS_REGEX_DFA_COLD".to_owned(), OsString::from("off")),
        ("ROMACS_REGEX_DFA".to_owned(), OsString::from("off")),
        (
            "ROMACS_REGEX_CACHE_DIAGNOSTIC".to_owned(),
            OsString::from("diagnostic.csv"),
        ),
    ]);
    let mut baseline = inherited.clone();
    let mut candidate = inherited.clone();
    policy(&["ROMACS_REGEX_DFA_COLD"]).apply_to(&mut baseline);
    policy(&["ROMACS_REGEX_SUFFIX_LITERAL=0", "ROMACS_REGEX_DFA=verify"]).apply_to(&mut candidate);
    assert!(!baseline.contains_key("ROMACS_REGEX_DFA_COLD"));
    assert_eq!(candidate["ROMACS_REGEX_DFA_COLD"], "off");
    assert_eq!(candidate["ROMACS_REGEX_SUFFIX_LITERAL"], "0");
    assert_eq!(candidate["ROMACS_REGEX_DFA"], "verify");
    assert_eq!(baseline["ROMACS_REGEX_DFA"], "off");
    assert_eq!(inherited["ROMACS_REGEX_DFA_COLD"], "off");
    assert!(!inherited.contains_key("ROMACS_REGEX_SUFFIX_LITERAL"));
    assert_eq!(candidate["ROMACS_REGEX_CACHE_DIAGNOSTIC"], "diagnostic.csv");
}

#[test]
fn invalid_or_duplicate_actions_fail_in_cli_and_artifacts() {
    for input in [
        "PATH=x",
        "ROMACS_REGEX_CACHE_DIAGNOSTIC=x",
        "ROMACS_REGEX_DFA_FIRST_STEP=maybe",
        "ROMACS_REGEX_DFA_COLD=OFF",
        "ROMACS_REGEX_DFA=0",
        "ROMACS_REGEX_DFA=-1",
        "ROMACS_REGEX_SUFFIX_LITERAL=4294967296",
        "ROMACS_REGEX_DFA_FIRST_STEP=",
    ] {
        assert!(input.parse::<ExecutionOverride>().is_err(), "{input}");
        assert!(
            serde_json::from_str::<ExecutionOverride>(&serde_json::to_string(input).unwrap())
                .is_err(),
            "{input}"
        );
    }
    let repeated = vec![
        "ROMACS_REGEX_DFA_FIRST_STEP=0".parse().unwrap(),
        "ROMACS_REGEX_DFA_FIRST_STEP".parse().unwrap(),
    ];
    assert!(ExecutionOverrides::try_from(repeated).is_err());
    assert!(
        serde_json::from_str::<ExecutionOverrides>(
            r#"["ROMACS_REGEX_DFA_FIRST_STEP=0","ROMACS_REGEX_DFA_FIRST_STEP"]"#
        )
        .is_err()
    );
}

#[test]
fn explicit_actions_and_forced_interpreter_must_match_provenance() {
    let overrides = policy(&["ROMACS_REGEX_DFA_COLD", "ROMACS_REGEX_SUFFIX_LITERAL=0"]);
    let mut recorded = BTreeMap::from([("ROMACS_REGEX_SUFFIX_LITERAL".to_owned(), "0".to_owned())]);
    assert!(overrides.validate_recorded(&recorded).is_ok());
    recorded.insert("ROMACS_REGEX_DFA_COLD".to_owned(), "off".to_owned());
    assert!(overrides.validate_recorded(&recorded).is_err());
    let encoded = serde_json::to_string(&overrides).unwrap();
    assert_eq!(
        serde_json::from_str::<ExecutionOverrides>(&encoded).unwrap(),
        overrides
    );
}

#[test]
fn regex_dfa_actions_preserve_values_and_validate_provenance() {
    for mode in ["off", "on", "verify"] {
        let input = format!("ROMACS_REGEX_DFA={mode}");
        let action: ExecutionOverride = input.parse().unwrap();
        assert_eq!(String::from(action.clone()), input);
        let encoded = serde_json::to_string(&action).unwrap();
        assert_eq!(
            serde_json::from_str::<ExecutionOverride>(&encoded).unwrap(),
            action
        );
    }
    let overrides = policy(&[
        "ROMACS_REGEX_DFA=verify",
        "ROMACS_REGEX_DFA_COLD=on",
        "ROMACS_REGEX_DFA_FIRST_STEP",
        "ROMACS_REGEX_SUFFIX_LITERAL=on",
    ]);
    let mut environment = BTreeMap::from([
        ("ROMACS_REGEX_DFA".to_owned(), OsString::from("off")),
        (
            "ROMACS_REGEX_DFA_FIRST_STEP".to_owned(),
            OsString::from("on"),
        ),
    ]);
    overrides.apply_to(&mut environment);
    assert_eq!(environment["ROMACS_REGEX_DFA"], "verify");
    assert_eq!(environment["ROMACS_REGEX_DFA_COLD"], "on");
    assert!(!environment.contains_key("ROMACS_REGEX_DFA_FIRST_STEP"));
    assert_eq!(environment["ROMACS_REGEX_SUFFIX_LITERAL"], "on");
    let mut recorded = environment
        .into_iter()
        .map(|(key, value)| (key, value.into_string().unwrap()))
        .collect::<BTreeMap<_, _>>();
    assert!(overrides.validate_recorded(&recorded).is_ok());
    recorded.insert("ROMACS_REGEX_DFA".to_owned(), "on".to_owned());
    assert!(overrides.validate_recorded(&recorded).is_err());
    for invalid in [
        "ROMACS_REGEX_DFA=maybe",
        "ROMACS_REGEX_DFA_COLD=verify",
        "ROMACS_REGEX_DFA_FIRST_STEP=verify",
        "ROMACS_REGEX_SUFFIX_LITERAL=verify",
    ] {
        assert!(invalid.parse::<ExecutionOverride>().is_err(), "{invalid}");
    }
}

#[test]
fn editing_cliff_actions_preserve_values_and_validate_provenance() {
    for name in [
        "ROMACS_COMPARE_STRINGS_POS_CACHE",
        "ROMACS_REGEX_SHORT_LITERAL",
        "ROMACS_EMACS_MULE_PREPARED",
        "ROMACS_OVERLAY_LOCAL_MOVE",
    ] {
        let mut environment = BTreeMap::new();
        let input = format!("{name}=on");
        let overrides = policy(&[&input]);
        overrides.apply_to(&mut environment);
        assert_eq!(environment[name], "on");
        let mut recorded = BTreeMap::from([(name.to_owned(), "on".to_owned())]);
        assert!(overrides.validate_recorded(&recorded).is_ok());
        recorded.insert(name.to_owned(), "off".to_owned());
        assert!(overrides.validate_recorded(&recorded).is_err());
        policy(&[name]).apply_to(&mut environment);
        assert!(!environment.contains_key(name));
        for value in ["off", "on"] {
            let action: ExecutionOverride = format!("{name}={value}").parse().unwrap();
            assert_eq!(String::from(action), format!("{name}={value}"));
        }
        for value in ["maybe", "verify", ""] {
            assert!(
                format!("{name}={value}")
                    .parse::<ExecutionOverride>()
                    .is_err()
            );
        }
    }
    // This encoder recognizes on/1/true. A boolean alias that it treats as
    // disabled must not be accepted as an enabling performance override.
    assert!(
        "ROMACS_EMACS_MULE_PREPARED=yes"
            .parse::<ExecutionOverride>()
            .is_err()
    );
}
