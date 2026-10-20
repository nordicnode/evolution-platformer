//! A hand-rolled argument parser.
//!
//! Deliberately dependency-free: `clap` would be the largest thing in a
//! 4-crate workspace whose whole selling point is a fast, tiny core, and the
//! surface here is a dozen flags. Options are collected in insertion order so
//! `--help` and error messages are stable.

use std::fmt;

/// What went wrong while parsing, and what to do about it.
pub enum ParseError {
    /// Print the help text and exit 0.
    HelpRequested,
    /// Print the version and exit 0.
    VersionRequested,
    /// Print the message plus a "try --help" hint and exit 2.
    Bad(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::HelpRequested => write!(f, ""),
            ParseError::VersionRequested => write!(f, ""),
            ParseError::Bad(m) => write!(f, "{m}"),
        }
    }
}

/// Parsed command line: a subcommand, `--key value` / `--key=value` options, and
/// bare boolean flags.
#[derive(Clone, Debug, Default)]
pub struct Args {
    pub command: String,
    opts: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Args {
    /// Parse `std::env::args()` minus the program name.
    pub fn parse(raw: &[String]) -> Result<Args, ParseError> {
        let mut command = String::new();
        let mut opts: Vec<(String, String)> = Vec::new();
        let mut flags: Vec<String> = Vec::new();

        let mut i = 0;
        while i < raw.len() {
            let arg = &raw[i];
            if arg == "--help" || arg == "-h" {
                return Err(ParseError::HelpRequested);
            }
            if arg == "--version" || arg == "-V" {
                return Err(ParseError::VersionRequested);
            }
            if let Some(rest) = arg.strip_prefix("--") {
                if let Some((k, v)) = rest.split_once('=') {
                    if k.is_empty() {
                        return Err(ParseError::Bad("empty option name".into()));
                    }
                    opts.push((k.to_string(), v.to_string()));
                    i += 1;
                    continue;
                }
                // Either a boolean flag or an option whose value is the next arg.
                let next_is_value = raw
                    .get(i + 1)
                    .map(|n| !n.starts_with("--"))
                    .unwrap_or(false);
                if next_is_value {
                    opts.push((rest.to_string(), raw[i + 1].clone()));
                    i += 2;
                } else {
                    flags.push(rest.to_string());
                    i += 1;
                }
                continue;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                return Err(ParseError::Bad(format!("unknown short option {arg:?}")));
            }
            if command.is_empty() {
                command = arg.clone();
            } else {
                return Err(ParseError::Bad(format!(
                    "unexpected positional argument {arg:?}; \
                     this CLI takes options only (values go after an option name)"
                )));
            }
            i += 1;
        }

        if command.is_empty() {
            return Err(ParseError::Bad("no subcommand given".into()));
        }
        Ok(Args {
            command,
            opts,
            flags,
        })
    }

    pub fn has_flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    pub fn opt(&self, name: &str) -> Option<&str> {
        // Last occurrence wins, so a caller can override a default by appending.
        self.opts
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn string(&self, name: &str, default: &str) -> String {
        self.opt(name).unwrap_or(default).to_string()
    }

    pub fn required_string(&self, name: &str) -> Result<String, ParseError> {
        self.opt(name)
            .map(|s| s.to_string())
            .ok_or_else(|| ParseError::Bad(format!("--{name} is required for `{}`", self.command)))
    }

    /// A required *value* option: `--name` alone is a usage error, not a default.
    pub fn required_path(&self, name: &str) -> Result<String, ParseError> {
        if self.opt(name).is_none() && self.has_flag(name) {
            return Err(ParseError::Bad(format!(
                "--{name} needs a value, e.g. --{name} levels/l1-flat-run.json"
            )));
        }
        self.required_string(name)
    }

    pub fn usize_opt(&self, name: &str, default: usize) -> Result<usize, ParseError> {
        match self.opt(name) {
            None => Ok(default),
            Some(v) => v.parse::<usize>().map_err(|_| {
                ParseError::Bad(format!("--{name} expects a positive integer, got {v:?}"))
            }),
        }
    }

    pub fn u64_opt(&self, name: &str, default: u64) -> Result<u64, ParseError> {
        match self.opt(name) {
            None => Ok(default),
            Some(v) => v
                .parse::<u64>()
                .map_err(|_| ParseError::Bad(format!("--{name} expects an integer, got {v:?}"))),
        }
    }

    pub fn f32_opt(&self, name: &str, default: f32) -> Result<f32, ParseError> {
        match self.opt(name) {
            None => Ok(default),
            Some(v) => {
                let parsed = v.parse::<f32>().map_err(|_| {
                    ParseError::Bad(format!("--{name} expects a number, got {v:?}"))
                })?;
                if !parsed.is_finite() {
                    return Err(ParseError::Bad(format!(
                        "--{name} must be finite, got {v:?}"
                    )));
                }
                Ok(parsed)
            }
        }
    }

    pub fn f64_opt(&self, name: &str, default: f64) -> Result<f64, ParseError> {
        match self.opt(name) {
            None => Ok(default),
            Some(v) => v
                .parse::<f64>()
                .map_err(|_| ParseError::Bad(format!("--{name} expects a number, got {v:?}"))),
        }
    }

    /// Reject unknown options so a typo fails loudly instead of silently doing
    /// nothing (the most annoying class of CLI bug there is).
    pub fn reject_unknown(&self, known: &[&str]) -> Result<(), ParseError> {
        for (k, _) in &self.opts {
            if !known.contains(&k.as_str()) {
                return Err(ParseError::Bad(format!(
                    "unknown option --{k} for `{}`",
                    self.command
                )));
            }
        }
        for f in &self.flags {
            if !known.contains(&f.as_str()) {
                return Err(ParseError::Bad(format!(
                    "unknown flag --{f} for `{}`",
                    self.command
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[&str]) -> Result<Args, ParseError> {
        let owned: Vec<String> = s.iter().map(|x| x.to_string()).collect();
        Args::parse(&owned)
    }

    #[test]
    fn long_options_take_the_next_argument_or_an_equals_form() {
        let a = parse(&["train", "--pop", "200", "--seed=7"])
            .ok()
            .expect("parses");
        assert_eq!(a.command, "train");
        assert_eq!(a.usize_opt("pop", 1).ok(), Some(200));
        assert_eq!(a.u64_opt("seed", 1).ok(), Some(7));
    }

    #[test]
    fn a_trailing_option_without_a_value_becomes_a_flag() {
        let a = parse(&["train", "--quiet"]).ok().expect("parses");
        assert!(a.has_flag("quiet"));
        assert_eq!(a.opt("quiet"), None);
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert!(matches!(parse(&["--help"]), Err(ParseError::HelpRequested)));
        assert!(matches!(parse(&["-h"]), Err(ParseError::HelpRequested)));
        assert!(matches!(
            parse(&["train", "--version"]),
            Err(ParseError::VersionRequested)
        ));
    }

    #[test]
    fn no_subcommand_is_an_error() {
        assert!(matches!(parse(&["--pop", "5"]), Err(ParseError::Bad(_))));
    }

    #[test]
    fn positional_arguments_are_rejected() {
        assert!(matches!(
            parse(&["train", "extra"]),
            Err(ParseError::Bad(_))
        ));
    }

    #[test]
    fn a_bad_number_is_reported_with_the_option_name() {
        match parse(&["train", "--pop", "abc"]) {
            Ok(a) => {
                let err = a.usize_opt("pop", 1).expect_err("must fail");
                assert!(format!("{err}").contains("--pop"));
            }
            Err(e) => panic!("should have parsed: {e}"),
        }
    }

    #[test]
    fn unknown_options_are_rejected_but_known_ones_pass() {
        let a = parse(&["train", "--pop", "10", "--typo", "1"])
            .ok()
            .expect("parses");
        assert!(a.reject_unknown(&["pop"]).is_err());
        assert!(a.reject_unknown(&["pop", "typo"]).is_ok());
    }

    #[test]
    fn last_occurrence_wins() {
        let a = parse(&["train", "--pop", "10", "--pop", "20"])
            .ok()
            .expect("parses");
        assert_eq!(a.usize_opt("pop", 1).ok(), Some(20));
    }
}
