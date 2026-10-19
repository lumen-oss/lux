use semver::{Comparator, Op};

use super::{comparator_component_count, PackageVersion, PackageVersionReq, SemVer};

impl PackageVersion {
    pub(crate) fn upgrade_version_req(&self, req: &PackageVersionReq) -> Option<String> {
        let PackageVersion::SemVer(sem_ver) = self else {
            return None;
        };
        if req.matches(self) {
            return None;
        }
        let PackageVersionReq::SemVer(version_req) = req else {
            return None;
        };
        let candidate = match version_req.comparators.as_slice() {
            [comparator] => upgrade_single_comparator(sem_ver, comparator),
            _ => range_bounds(&version_req.comparators)
                .and_then(|(lower, upper)| upgrade_bounded_range(sem_ver, lower, upper)),
        }?;
        validated_candidate(candidate, self)
    }
}

fn validated_candidate(candidate: String, version: &PackageVersion) -> Option<String> {
    PackageVersionReq::parse(&candidate)
        .ok()
        .filter(|parsed| parsed.matches(version))
        .map(|_| candidate)
}

fn upgrade_single_comparator(sem_ver: &SemVer, comparator: &Comparator) -> Option<String> {
    let operand = version_operand(sem_ver, comparator_component_count(comparator));
    match comparator.op {
        Op::Exact => Some(operand),
        Op::Tilde => Some(format!("~{operand}")),
        Op::Caret => Some(format!("^{operand}")),
        Op::Less | Op::LessEq => Some(upgrade_upper_bound(sem_ver, comparator)),
        _ => None,
    }
}

fn upgrade_range_comparator(
    sem_ver: &SemVer,
    min: &Comparator,
    max: &Comparator,
) -> Option<String> {
    Some(format!(
        "~> {}",
        version_operand(sem_ver, range_component_count(min, max))
    ))
}

fn upgrade_bounded_range(
    sem_ver: &SemVer,
    lower: &Comparator,
    upper: &Comparator,
) -> Option<String> {
    if lower.op == Op::GreaterEq && upper.op == Op::Less {
        upgrade_range_comparator(sem_ver, lower, upper)
    } else {
        upgrade_explicit_range(sem_ver, lower, upper)
    }
}

fn upgrade_explicit_range(
    sem_ver: &SemVer,
    lower: &Comparator,
    upper: &Comparator,
) -> Option<String> {
    Some(format!(
        "{} {}, {}",
        comparator_op(lower),
        comparator_operand(lower),
        upgrade_upper_bound(sem_ver, upper)
    ))
}

fn upgrade_upper_bound(sem_ver: &SemVer, upper: &Comparator) -> String {
    let component_count = comparator_component_count(upper);
    let operand = match upper.op {
        Op::Less => next_version_operand(sem_ver, component_count),
        _ => version_operand(sem_ver, component_count),
    };
    format!("{} {operand}", comparator_op(upper))
}

fn next_version_operand(sem_ver: &SemVer, component_count: usize) -> String {
    match component_count {
        1 => (sem_ver.version.major + 1).to_string(),
        2 => format!("{}.{}", sem_ver.version.major, sem_ver.version.minor + 1),
        _ => format!(
            "{}.{}.{}",
            sem_ver.version.major,
            sem_ver.version.minor,
            sem_ver.version.patch + 1
        ),
    }
}

fn range_bounds(comparators: &[Comparator]) -> Option<(&Comparator, &Comparator)> {
    match comparators {
        [lower, upper] if is_lower_bound(lower) && is_upper_bound(upper) => Some((lower, upper)),
        [upper, lower] if is_upper_bound(upper) && is_lower_bound(lower) => Some((lower, upper)),
        _ => None,
    }
}

fn is_lower_bound(comparator: &Comparator) -> bool {
    matches!(comparator.op, Op::GreaterEq | Op::Greater)
}

fn is_upper_bound(comparator: &Comparator) -> bool {
    matches!(comparator.op, Op::Less | Op::LessEq)
}

fn comparator_op(comparator: &Comparator) -> &'static str {
    match comparator.op {
        Op::GreaterEq => ">=",
        Op::Greater => ">",
        Op::LessEq => "<=",
        Op::Less => "<",
        _ => "",
    }
}

fn comparator_operand(comparator: &Comparator) -> String {
    match comparator_component_count(comparator) {
        1 => comparator.major.to_string(),
        2 => format!("{}.{}", comparator.major, comparator.minor.unwrap_or(0)),
        _ => format!(
            "{}.{}.{}",
            comparator.major,
            comparator.minor.unwrap_or(0),
            comparator.patch.unwrap_or(0)
        ),
    }
}

fn version_operand(sem_ver: &SemVer, component_count: usize) -> String {
    match component_count {
        1 => sem_ver.version.major.to_string(),
        2 => format!("{}.{}", sem_ver.version.major, sem_ver.version.minor),
        _ => sem_ver.version.to_string(),
    }
}

fn range_component_count(min: &Comparator, max: &Comparator) -> usize {
    if min.major != max.major {
        1
    } else if min.minor != max.minor {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn upgrade_version_req() {
        for (req, latest, expected) in [
            ("~> 1", "2.0.0", Some("~> 2")),
            ("~> 1.0", "1.1.0", Some("~> 1.1")),
            ("~> 1.0.0", "1.1.1", Some("~> 1.1.1")),
            ("~> 1.0.0", "1.5.6-2", Some("~> 1.5.6")),
            ("~> 0.5", "0.5.9", None),
            ("~> 0.5", "0.6.0", Some("~> 0.6")),
            ("~> 1", "1.9.0", None),
            ("1.0.0", "2.0.0", Some("2.0.0")),
            ("==1.0.0", "2.0.0", Some("2.0.0")),
            ("1", "2.0.0", Some("2")),
            ("1.0", "1.5.0", Some("1.5")),
            ("~1.2", "1.5.0", Some("~1.5")),
            ("~1.2.3", "1.5.6", Some("~1.5.6")),
            ("~1.2", "1.2.5", None),
            ("^1.2", "2.0.0", Some("^2.0")),
            ("^1.2.3", "2.0.0", Some("^2.0.0")),
            ("^0.2.3", "0.3.0", Some("^0.3.0")),
            (">= 1.0, < 2.0", "2.3.0", Some("~> 2")),
            ("< 2.0, >= 1.0", "2.3.0", Some("~> 2")),
            (">= 1.0, <= 2.0", "2.3.0", Some(">= 1.0, <= 2.3")),
            ("<= 2.0, >= 1.0", "2.3.0", Some(">= 1.0, <= 2.3")),
            ("> 1.0, <= 2.0", "2.3.0", Some("> 1.0, <= 2.3")),
            (">= 1.0.0, <= 2.0.0", "2.3.0", Some(">= 1.0.0, <= 2.3.0")),
            ("> 1.0, < 2.0", "2.3.0", Some("> 1.0, < 2.4")),
            ("> 1, < 2", "2.3.0", Some("> 1, < 3")),
            ("> 1.0.0, < 2.0.0", "2.3.0", Some("> 1.0.0, < 2.3.1")),
            (">= 1.0.0", "2.0.0", None),
            (">= 1.0.0", "0.5.0", None),
            ("> 1.0.0", "2.0.0", None),
            ("< 2.0.0", "2.0.0", Some("< 2.0.1")),
            ("< 2.0.0", "2.3.0", Some("< 2.3.1")),
            ("<= 2.0.0", "2.0.0", None),
            ("<= 2.0.0", "2.3.0", Some("<= 2.3.0")),
            ("< 2", "2.3.0", Some("< 3")),
            ("<= 2", "3.0.0", Some("<= 3")),
            ("*", "2.0.0", None),
        ] {
            let req = PackageVersionReq::parse(req).unwrap();
            let latest = PackageVersion::parse(latest).unwrap();
            assert_eq!(
                latest.upgrade_version_req(&req).as_deref(),
                expected,
                "req {req}, latest {latest}"
            );
        }

        assert_eq!(
            PackageVersion::parse("scm")
                .unwrap()
                .upgrade_version_req(&PackageVersionReq::parse("~> 1").unwrap()),
            None
        );
    }

    fn semver_strategy() -> impl Strategy<Value = PackageVersion> {
        let pre = prop::option::of(prop_oneof![
            Just("1".to_string()),
            Just("alpha.1".to_string())
        ]);
        (0u64..4, 0u64..4, 0u64..4, pre).prop_map(|(major, minor, patch, pre)| {
            let version = match pre {
                Some(pre) => format!("{major}.{minor}.{patch}-{pre}"),
                None => format!("{major}.{minor}.{patch}"),
            };
            PackageVersion::parse(&version).unwrap()
        })
    }

    fn supported_req_strategy() -> impl Strategy<Value = PackageVersionReq> {
        let op = prop_oneof![Just("~>"), Just("~"), Just("^"), Just("=="), Just("")];
        (op, 0u64..4, 0u64..4, 0u64..4, 1usize..=3).prop_map(
            |(op, major, minor, patch, precision)| {
                let num = [major, minor, patch]
                    .iter()
                    .take(precision)
                    .map(|component| component.to_string())
                    .collect::<Vec<_>>()
                    .join(".");
                let req = if op == "~>" {
                    format!("~> {num}")
                } else {
                    format!("{op}{num}")
                };
                PackageVersionReq::parse(&req).unwrap()
            },
        )
    }

    fn non_upgradable_req_strategy() -> impl Strategy<Value = PackageVersionReq> {
        let op = prop_oneof![Just(">="), Just(">"), Just("*")];
        (op, 0u64..4, 0u64..4, 0u64..4).prop_map(|(op, major, minor, patch)| {
            let req = if op == "*" {
                op.to_string()
            } else {
                format!("{op} {major}.{minor}.{patch}")
            };
            PackageVersionReq::parse(&req).unwrap()
        })
    }

    fn single_upper_bound_strategy() -> impl Strategy<Value = PackageVersionReq> {
        let op = prop_oneof![Just("<"), Just("<=")];
        (op, 0u64..4, 0u64..4, 0u64..4, 1usize..=3).prop_map(
            |(op, major, minor, patch, precision)| {
                let num = [major, minor, patch]
                    .iter()
                    .take(precision)
                    .map(|component| component.to_string())
                    .collect::<Vec<_>>()
                    .join(".");
                PackageVersionReq::parse(&format!("{op} {num}")).unwrap()
            },
        )
    }

    fn pessimistic_range_strategy() -> impl Strategy<Value = PackageVersionReq> {
        let reversed = any::<bool>();
        (
            reversed,
            0u64..4,
            0u64..4,
            0u64..4,
            0u64..4,
            0u64..4,
            0u64..4,
        )
            .prop_map(|(reversed, major, minor, patch, major2, minor2, patch2)| {
                let lower = format!(">= {major}.{minor}.{patch}");
                let upper = format!("< {major2}.{minor2}.{patch2}");
                let req = if reversed {
                    format!("{upper}, {lower}")
                } else {
                    format!("{lower}, {upper}")
                };
                PackageVersionReq::parse(&req).unwrap()
            })
    }

    fn explicit_range_strategy() -> impl Strategy<Value = PackageVersionReq> {
        let shape = prop_oneof![
            (Just(">="), Just("<=")),
            (Just(">"), Just("<")),
            (Just(">"), Just("<=")),
        ];
        let reversed = any::<bool>();
        (
            shape,
            reversed,
            0u64..4,
            0u64..4,
            0u64..4,
            0u64..4,
            0u64..4,
            0u64..4,
        )
            .prop_map(
                |((lower_op, upper_op), reversed, major, minor, patch, major2, minor2, patch2)| {
                    let lower = format!("{lower_op} {major}.{minor}.{patch}");
                    let upper = format!("{upper_op} {major2}.{minor2}.{patch2}");
                    let req = if reversed {
                        format!("{upper}, {lower}")
                    } else {
                        format!("{lower}, {upper}")
                    };
                    PackageVersionReq::parse(&req).unwrap()
                },
            )
    }

    fn pessimistic_range(comparators: &[Comparator]) -> bool {
        matches!(
            range_bounds(comparators),
            Some((lower, upper)) if lower.op == Op::GreaterEq && upper.op == Op::Less
        )
    }

    fn range_shape(req: &PackageVersionReq) -> Option<(Comparator, &'static str, usize)> {
        let PackageVersionReq::SemVer(version_req) = req else {
            return None;
        };
        let (lower, upper) = range_bounds(&version_req.comparators)?;
        Some((
            lower.clone(),
            comparator_op(upper),
            comparator_component_count(upper),
        ))
    }

    fn single_shape(req: &PackageVersionReq) -> Option<(&'static str, usize)> {
        let PackageVersionReq::SemVer(version_req) = req else {
            return None;
        };
        let [comparator] = version_req.comparators.as_slice() else {
            return None;
        };
        Some((
            comparator_op(comparator),
            comparator_component_count(comparator),
        ))
    }

    fn req_operator(req: &PackageVersionReq) -> &'static str {
        let PackageVersionReq::SemVer(version_req) = req else {
            return "";
        };
        match version_req.comparators.as_slice() {
            [comparator] => match comparator.op {
                Op::Tilde => "~",
                Op::Caret => "^",
                _ => "",
            },
            _ if pessimistic_range(&version_req.comparators) => "~>",
            _ => "",
        }
    }

    fn output_operator(req: &str) -> &'static str {
        if req.starts_with("~>") {
            "~>"
        } else if req.starts_with('~') {
            "~"
        } else if req.starts_with('^') {
            "^"
        } else {
            ""
        }
    }

    proptest! {
        #[test]
        fn upgrade_admits_latest(
            req in prop_oneof![
                supported_req_strategy(),
                pessimistic_range_strategy(),
                explicit_range_strategy(),
                single_upper_bound_strategy(),
            ],
            latest in semver_strategy(),
        ) {
            if let Some(upgraded) = latest.upgrade_version_req(&req) {
                let parsed = PackageVersionReq::parse(&upgraded).unwrap();
                prop_assert!(parsed.matches(&latest));
            }
        }

        #[test]
        fn upgrade_is_idempotent(
            req in prop_oneof![
                supported_req_strategy(),
                pessimistic_range_strategy(),
                explicit_range_strategy(),
                single_upper_bound_strategy(),
            ],
            latest in semver_strategy(),
        ) {
            if let Some(upgraded) = latest.upgrade_version_req(&req) {
                let parsed = PackageVersionReq::parse(&upgraded).unwrap();
                prop_assert_eq!(latest.upgrade_version_req(&parsed), None);
            }
        }

        #[test]
        fn non_upgradable_req_is_unchanged(
            req in non_upgradable_req_strategy(),
            latest in semver_strategy(),
        ) {
            prop_assert_eq!(latest.upgrade_version_req(&req), None);
        }

        #[test]
        fn operator_is_preserved(
            req in supported_req_strategy(),
            latest in semver_strategy(),
        ) {
            if let Some(upgraded) = latest.upgrade_version_req(&req) {
                prop_assert_eq!(output_operator(&upgraded), req_operator(&req));
            }
        }

        #[test]
        fn pessimistic_range_becomes_pessimistic(
            req in pessimistic_range_strategy(),
            latest in semver_strategy(),
        ) {
            if let Some(upgraded) = latest.upgrade_version_req(&req) {
                prop_assert!(upgraded.starts_with("~>"));
            }
        }

        #[test]
        fn explicit_range_preserves_bounds(
            req in explicit_range_strategy(),
            latest in semver_strategy(),
        ) {
            if let Some(upgraded) = latest.upgrade_version_req(&req) {
                let parsed = PackageVersionReq::parse(&upgraded).unwrap();
                prop_assert_eq!(range_shape(&parsed), range_shape(&req));
            }
        }

        #[test]
        fn single_upper_bound_preserves_bounds(
            req in single_upper_bound_strategy(),
            latest in semver_strategy(),
        ) {
            if let Some(upgraded) = latest.upgrade_version_req(&req) {
                let parsed = PackageVersionReq::parse(&upgraded).unwrap();
                prop_assert_eq!(single_shape(&parsed), single_shape(&req));
            }
        }
    }
}
