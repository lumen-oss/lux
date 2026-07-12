//! Utility module for converting clap parsing errors into good-looking [`miette`] errors
use std::{error::Error, ffi::OsStr, path::Path};

use clap::error::{ContextKind, ContextValue, ErrorKind};
use miette::{LabeledSpan, MietteDiagnostic, NamedSource, SourceSpan};

/// All the [`ContextKind`] data exposed by a [`clap::Error`], extracted into a
/// struct so we can pattern-match on it freely.
enum ErrorContext {
    InvalidValue {
        arg: Option<String>,
        value: Option<String>,
        valid_values: Vec<String>,
        custom: Option<String>,
    },
    ValueValidation {
        arg: Option<String>,
        value: Option<String>,
        custom: Option<String>,
    },
    UnknownArgument {
        arg: Option<String>,
        suggested_arg: Option<String>,
    },
    InvalidSubcommand {
        subcommand: Option<String>,
        suggested_subcommand: Option<String>,
        valid_subcommands: Vec<String>,
    },
    MissingRequiredArgument {
        arg: Option<String>,
    },
    MissingSubcommand,
    NoEquals {
        arg: Option<String>,
    },
    TooManyValues {
        arg: Option<String>,
        actual: Option<isize>,
        expected: Option<isize>,
        custom: Option<String>,
    },
    TooFewValues {
        arg: Option<String>,
        actual: Option<isize>,
        expected: Option<isize>,
        custom: Option<String>,
    },
    WrongNumberOfValues {
        arg: Option<String>,
        actual: Option<isize>,
        expected: Option<isize>,
        custom: Option<String>,
    },
    ArgumentConflict {
        arg: Option<String>,
        prior: Vec<String>,
    },
    InvalidUtf8,
    Other(String),
}

impl ErrorContext {
    fn extract(error: &clap::Error) -> Self {
        let kind = error.kind();
        let mut custom = error.source().map(|s| s.to_string());

        // Capture all the possibilities upfront so we can package it in structs later
        let mut invalid_arg: Option<String> = None;
        let mut invalid_value: Option<String> = None;
        let mut invalid_subcommand: Option<String> = None;
        let mut suggested_arg: Option<String> = None;
        let mut suggested_subcommand: Option<String> = None;
        let mut valid_values: Vec<String> = Vec::new();
        let mut valid_subcommands: Vec<String> = Vec::new();
        let mut prior_arg: Vec<String> = Vec::new();
        let mut actual_num: Option<isize> = None;
        let mut expected_num: Option<isize> = None;
        let mut min_values: Option<isize> = None;

        for (ctx_kind, value) in error.context() {
            match (ctx_kind, value) {
                (ContextKind::InvalidArg, ContextValue::String(s)) => {
                    invalid_arg = Some(flag_of(s));
                }
                (ContextKind::InvalidArg, ContextValue::Strings(ss)) => {
                    if let Some(s) = ss.first() {
                        invalid_arg = Some(flag_of(s));
                    }
                }
                (ContextKind::InvalidValue, ContextValue::String(s)) => {
                    invalid_value = Some(s.clone());
                }
                (ContextKind::InvalidSubcommand, ContextValue::String(s)) => {
                    invalid_subcommand = Some(s.clone());
                }
                (ContextKind::SuggestedArg, ContextValue::String(s)) => {
                    suggested_arg = Some(s.clone());
                }
                (ContextKind::SuggestedSubcommand, ContextValue::Strings(ss)) => {
                    if let Some(first) = ss.first() {
                        suggested_subcommand = Some(first.clone());
                    }
                }
                (ContextKind::ValidValue, ContextValue::Strings(ss)) => {
                    valid_values = ss.clone();
                }
                (ContextKind::ValidSubcommand, ContextValue::Strings(ss)) => {
                    valid_subcommands = ss.clone();
                }
                (ContextKind::PriorArg, ContextValue::String(s)) => prior_arg.push(s.clone()),
                (ContextKind::PriorArg, ContextValue::Strings(ss)) => {
                    prior_arg.extend(ss.iter().cloned());
                }
                (ContextKind::ActualNumValues, ContextValue::Number(n)) => {
                    actual_num = Some(*n);
                }
                (ContextKind::ExpectedNumValues, ContextValue::Number(n)) => {
                    expected_num = Some(*n);
                }
                (ContextKind::MinValues, ContextValue::Number(n)) => min_values = Some(*n),
                (ContextKind::Custom, ContextValue::String(s)) => {
                    custom.get_or_insert_with(|| s.clone());
                }
                _ => {}
            }
        }

        match kind {
            ErrorKind::InvalidValue => Self::InvalidValue {
                arg: invalid_arg,
                value: invalid_value,
                valid_values,
                custom,
            },
            ErrorKind::ValueValidation => Self::ValueValidation {
                arg: invalid_arg,
                value: invalid_value,
                custom,
            },
            ErrorKind::UnknownArgument => Self::UnknownArgument {
                arg: invalid_arg,
                suggested_arg,
            },
            ErrorKind::InvalidSubcommand => Self::InvalidSubcommand {
                subcommand: invalid_subcommand,
                suggested_subcommand,
                valid_subcommands,
            },
            ErrorKind::MissingRequiredArgument => {
                Self::MissingRequiredArgument { arg: invalid_arg }
            }
            ErrorKind::MissingSubcommand => Self::MissingSubcommand,
            ErrorKind::NoEquals => Self::NoEquals { arg: invalid_arg },
            ErrorKind::TooManyValues => Self::TooManyValues {
                arg: invalid_arg,
                actual: actual_num,
                expected: expected_num,
                custom,
            },
            ErrorKind::TooFewValues => Self::TooFewValues {
                arg: invalid_arg,
                actual: actual_num,
                expected: expected_num.or(min_values),
                custom,
            },
            ErrorKind::WrongNumberOfValues => Self::WrongNumberOfValues {
                arg: invalid_arg,
                actual: actual_num,
                expected: expected_num,
                custom,
            },
            ErrorKind::ArgumentConflict => Self::ArgumentConflict {
                arg: invalid_arg,
                prior: prior_arg,
            },
            ErrorKind::InvalidUtf8 => Self::InvalidUtf8,
            _ => Self::Other(error.to_string()),
        }
    }

    fn arg_extract(&self) -> Option<&str> {
        match self {
            Self::InvalidValue { arg, .. }
            | Self::ValueValidation { arg, .. }
            | Self::UnknownArgument { arg, .. }
            | Self::MissingRequiredArgument { arg }
            | Self::NoEquals { arg }
            | Self::TooManyValues { arg, .. }
            | Self::TooFewValues { arg, .. }
            | Self::WrongNumberOfValues { arg, .. }
            | Self::ArgumentConflict { arg, .. } => arg.as_deref(),
            _ => None,
        }
    }

    fn message(&self) -> String {
        let a = || self.arg_extract().unwrap_or("?");
        match self {
            Self::InvalidValue { value, .. } | Self::ValueValidation { value, .. } => {
                format!(
                    "invalid value `{}` for `{}`",
                    value.as_deref().unwrap_or("?"),
                    a()
                )
            }
            Self::UnknownArgument { .. } => format!("unexpected argument `{}`", a()),
            Self::InvalidSubcommand { subcommand, .. } => {
                format!(
                    "unknown subcommand `{}`",
                    subcommand.as_deref().unwrap_or("?")
                )
            }
            Self::MissingRequiredArgument { .. } => format!("missing required argument `{}`", a()),
            Self::MissingSubcommand => "missing required subcommand".to_string(),
            Self::NoEquals { .. } => format!("argument `{}` requires a value with `=`", a()),
            Self::TooManyValues { .. } => format!("too many values for `{}`", a()),
            Self::TooFewValues { .. } => format!("too few values for `{}`", a()),
            Self::WrongNumberOfValues { .. } => format!("wrong number of values for `{}`", a()),
            Self::ArgumentConflict { .. } => {
                format!("argument `{}` conflicts with a previous argument", a())
            }
            Self::InvalidUtf8 => "invalid UTF-8 in command-line arguments".to_string(),
            Self::Other(msg) => msg.clone(),
        }
    }

    fn help(&self) -> Option<String> {
        let hint = match self {
            Self::InvalidValue { valid_values, .. } if !valid_values.is_empty() => {
                Some(format!("possible values: {}", valid_values.join(", ")))
            }
            Self::InvalidSubcommand {
                suggested_subcommand,
                valid_subcommands,
                ..
            } => suggested_subcommand
                .as_ref()
                .map(|sug| format!("did you mean `{sug}`?"))
                .or_else(|| {
                    (!valid_subcommands.is_empty())
                        .then(|| format!("available subcommands: {}", valid_subcommands.join(", ")))
                }),
            Self::UnknownArgument { suggested_arg, .. } => suggested_arg
                .as_ref()
                .map(|sug| format!("did you mean `{sug}`?")),
            Self::ArgumentConflict { prior, .. } if !prior.is_empty() => {
                Some(format!("cannot be used with `{}`", prior.join("`, `")))
            }
            Self::TooManyValues {
                actual, expected, ..
            }
            | Self::TooFewValues {
                actual, expected, ..
            }
            | Self::WrongNumberOfValues {
                actual, expected, ..
            } => expected.and_then(|e| {
                let value_str = if e == 1 { "value" } else { "values" };
                actual.map(|a| format!("expected {e} {}, got {a}", value_str))
            }),
            Self::NoEquals { arg } => Some(format!(
                "use `{}=<value>` syntax",
                arg.as_deref().unwrap_or("...")
            )),
            _ => None,
        };
        let parts: Vec<_> = [hint, self.custom()].into_iter().flatten().collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }

    fn custom(&self) -> Option<String> {
        match self {
            Self::InvalidValue { custom, .. }
            | Self::ValueValidation { custom, .. }
            | Self::TooManyValues { custom, .. }
            | Self::TooFewValues { custom, .. }
            | Self::WrongNumberOfValues { custom, .. } => custom.clone(),
            _ => None,
        }
    }

    fn primary_label(&self, source: &str) -> Option<LabeledSpan> {
        match self {
            Self::InvalidValue { value, arg, .. } | Self::ValueValidation { value, arg, .. } => {
                value.as_deref().and_then(|value| {
                    let start = arg
                        .as_deref()
                        .and_then(|anchor| source.find(anchor).map(|p| p + anchor.len()))
                        .unwrap_or(0);
                    source[start..].find(value).map(|offset| {
                        LabeledSpan::new_primary_with_span(
                            Some("this value is invalid".into()),
                            SourceSpan::from((start + offset, value.len())),
                        )
                    })
                })
            }
            Self::UnknownArgument { .. } => {
                arg_span(source, self.arg_extract(), "unknown argument")
            }
            Self::InvalidSubcommand { subcommand, .. } => span_of(source, subcommand.as_deref()?)
                .map(|span| {
                    LabeledSpan::new_primary_with_span(Some("unknown subcommand".into()), span)
                }),
            Self::MissingRequiredArgument { .. }
            | Self::TooManyValues { .. }
            | Self::TooFewValues { .. }
            | Self::WrongNumberOfValues { .. } => {
                arg_span(source, self.arg_extract(), "this argument")
            }
            Self::MissingSubcommand => Some(LabeledSpan::new_primary_with_span(
                Some("expected a subcommand here".into()),
                SourceSpan::from((source.len(), 0)),
            )),
            Self::NoEquals { .. } => arg_span(source, self.arg_extract(), "missing `=`"),
            Self::ArgumentConflict { .. } => {
                arg_span(source, self.arg_extract(), "conflicting argument")
            }
            _ => None,
        }
    }

    fn secondary_label(&self, source: &str) -> Option<LabeledSpan> {
        match self {
            Self::InvalidValue { .. } | Self::ValueValidation { .. } => {
                self.arg_extract().and_then(|arg| {
                    span_of(source, arg).map(|span| {
                        LabeledSpan::new_with_span(Some("for this argument".into()), span)
                    })
                })
            }
            Self::UnknownArgument { suggested_arg, .. } => {
                suggested_arg.as_deref().and_then(|sug| {
                    span_of(source, sug).map(|span| {
                        LabeledSpan::new_with_span(Some(format!("did you mean `{sug}`?")), span)
                    })
                })
            }
            Self::InvalidSubcommand {
                suggested_subcommand,
                ..
            } => suggested_subcommand.as_deref().and_then(|sug| {
                span_of(source, sug).map(|span| {
                    LabeledSpan::new_with_span(Some(format!("did you mean `{sug}`?")), span)
                })
            }),
            _ => None,
        }
    }
}

fn span_of(source: &str, needle: &str) -> Option<SourceSpan> {
    source
        .find(needle)
        .map(|start| SourceSpan::from((start, needle.len())))
}

fn arg_span(source: &str, arg: Option<&str>, label: &str) -> Option<LabeledSpan> {
    arg.and_then(|a| {
        span_of(source, a).map(|span| LabeledSpan::new_primary_with_span(Some(label.into()), span))
    })
}

/// Strips value from key-value pair: `--lua-version <VER>` -> `--lua-version`
fn flag_of(s: &str) -> String {
    match s.find(' ') {
        Some(idx) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

/// Reconstruct the user's command-line invocation as a single string suitable
/// for use as a miette source. Example: `lx --lua-version 5.4 build`. This lets
/// us generate beautiful diagnostics.
fn build_source() -> String {
    let mut args = std::env::args_os();

    args.next()
        .map(|exe_path| {
            Path::new(&exe_path)
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or(exe_path)
        })
        .into_iter()
        .chain(args)
        .map(|a| shell_escape(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_escape(arg: &OsStr) -> String {
    shlex::try_quote(&arg.to_string_lossy())
        .unwrap_or_else(|err| {
            unreachable!(
                "Empty value during quoting. This is a bug. Error: {:?}",
                err
            )
        })
        .into_owned()
}

pub fn clap_to_miette(error: clap::Error) -> Result<miette::Report, String> {
    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            | ErrorKind::DisplayVersion
    ) {
        // pass through the error so it gets rendered normally
        return Err(error.to_string());
    }
    Ok(clap_to_miette_with_source(error, &build_source()))
}

fn clap_to_miette_with_source(error: clap::Error, source: &str) -> miette::Report {
    let source = source.to_string();
    let ctx = ErrorContext::extract(&error);
    let message = ctx.message();
    let labels: Vec<_> = [ctx.primary_label(&source), ctx.secondary_label(&source)]
        .into_iter()
        .flatten()
        .collect();
    let mut diag = MietteDiagnostic::new(message);
    if let Some(h) = ctx.help() {
        diag = diag.with_help(h);
    }
    if !labels.is_empty() {
        diag = diag.with_labels(labels);
    }
    miette::Report::new(diag).with_source_code(NamedSource::new("command", source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{Parser, Subcommand};
    use miette::SourceSpan;

    #[derive(Parser, Debug)]
    #[allow(dead_code)]
    struct TestCli {
        #[arg(long, value_parser = ["text", "json"])]
        format: Option<String>,
        #[arg(long, value_parser = parse_int)]
        timeout: Option<u32>,
        #[arg(long, num_args = 2..)]
        names: Vec<String>,
        #[arg(long, conflicts_with = "format")]
        server: Option<String>,
    }

    fn parse_int(s: &str) -> Result<u32, String> {
        s.parse().map_err(|_| format!("`{s}` is not a number"))
    }

    fn convert(args: &[&str], source: &str) -> miette::Report {
        let err = TestCli::try_parse_from(args).expect_err("expected parse error");
        clap_to_miette_with_source(err, source)
    }

    fn get_label<'a>(span: &SourceSpan, source: &'a str) -> &'a str {
        &source[span.offset()..span.offset() + span.len()]
    }

    #[test]
    fn unknown_argument_with_suggestion() {
        let report = convert(&["prog", "--forma"], "lx build --forma");
        let s = report.to_string();
        assert!(s.contains("unexpected argument"), "got: {s}");
        assert!(s.contains("--forma"), "got: {s}");
        let labels: Vec<_> = report.labels().unwrap().collect();
        assert!(labels[0].primary(), "first label should be primary");
        assert_eq!(get_label(labels[0].inner(), "lx build --forma"), "--forma");
        assert_eq!(
            report.help().map(|h| h.to_string()),
            Some("did you mean `--format`?".into())
        );
    }

    #[test]
    fn invalid_value_with_possible_values() {
        let report = convert(&["prog", "--format", "yaml"], "lx --format yaml");
        let s = report.to_string();
        assert!(s.contains("invalid value `yaml`"));
        assert!(s.contains("--format"));
        let labels: Vec<_> = report.labels().unwrap().collect();
        assert!(labels[0].primary());
        assert_eq!(get_label(labels[0].inner(), "lx --format yaml"), "yaml");
        assert_eq!(get_label(labels[1].inner(), "lx --format yaml"), "--format");
        assert_eq!(
            report.help().map(|h| h.to_string()),
            Some("possible values: text, json".into())
        );
    }

    #[test]
    fn invalid_value_with_custom_validation() {
        let report = convert(&["prog", "--timeout", "abc"], "lx --timeout abc");
        let s = report.to_string();
        assert!(s.contains("invalid value `abc`"));
        assert!(s.contains("--timeout"));
        let labels: Vec<_> = report.labels().unwrap().collect();
        assert_eq!(get_label(labels[0].inner(), "lx --timeout abc"), "abc");
        // The value_parser's custom error becomes a help note.
        assert!(report
            .help()
            .unwrap()
            .to_string()
            .contains("`abc` is not a number"));
    }

    #[test]
    fn invalid_subcommand_with_suggestion() {
        #[derive(Parser, Debug)]
        struct SubCli {
            #[command(subcommand)]
            cmd: SubCmd,
        }
        #[derive(Subcommand, Debug)]
        enum SubCmd {
            Install,
            Build,
        }

        let err =
            SubCli::try_parse_from(["prog", "instlal"]).expect_err("expected invalid subcommand");
        let report = clap_to_miette_with_source(err, "lx instlal");
        let s = report.to_string();
        assert!(s.contains("unknown subcommand `instlal`"), "got: {s}");
        let labels: Vec<_> = report.labels().unwrap().collect();
        assert!(labels[0].primary());
        assert_eq!(get_label(labels[0].inner(), "lx instlal"), "instlal");
        assert_eq!(
            report.help().map(|h| h.to_string()),
            Some("did you mean `install`?".into())
        );
    }

    #[test]
    fn too_few_values() {
        let report = convert(&["prog", "--names", "only-one"], "lx --names only-one");
        let s = report.to_string();
        assert!(s.contains("too few values for `--names`"));
        let labels: Vec<_> = report.labels().unwrap().collect();
        assert!(labels[0].primary());
        assert_eq!(
            get_label(labels[0].inner(), "lx --names only-one"),
            "--names"
        );
        assert!(report
            .help()
            .unwrap()
            .to_string()
            .contains("expected 2 values"));
    }

    #[test]
    fn missing_required_argument() {
        #[derive(Parser, Debug)]
        struct NeedArg {
            #[command(subcommand)]
            cmd: NeedArgCmd,
        }
        #[derive(Subcommand, Debug)]
        enum NeedArgCmd {
            Run {
                #[arg(long, required = true)]
                target: String,
            },
        }

        let err = NeedArg::try_parse_from(["prog", "run"]).expect_err("expected missing arg error");
        let report = clap_to_miette_with_source(err, "lx run");
        let s = report.to_string();
        assert!(s.contains("missing required argument"));
        assert!(s.contains("--target"));
    }

    #[test]
    fn argument_conflict() {
        let report = convert(
            &["prog", "--format", "text", "--server", "x"],
            "lx --format text --server x",
        );
        let s = report.to_string();
        assert!(s.contains("conflicts with"));
        let labels: Vec<_> = report.labels().unwrap().collect();
        assert!(labels[0].primary());
        assert_eq!(
            get_label(labels[0].inner(), "lx --format text --server x"),
            "--format"
        );
    }

    #[test]
    fn display_help_passes_through() {
        for kind in [
            ErrorKind::DisplayHelp,
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
            ErrorKind::DisplayVersion,
        ] {
            let err = clap::Error::new(kind);
            assert!(
                clap_to_miette(err).is_err(),
                "{kind:?} should bypass miette"
            );
        }
    }

    #[test]
    fn label_finds_value_after_flag() {
        let report = convert(&["prog", "--format", "yaml"], "yaml lx --format yaml");
        let labels: Vec<_> = report.labels().unwrap().collect();
        let primary_text = get_label(labels[0].inner(), "yaml lx --format yaml");
        assert_eq!(primary_text, "yaml");
        assert!(labels[0].offset() >= "yaml lx --format ".len());
    }
}
