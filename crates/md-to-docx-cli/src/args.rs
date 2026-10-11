//! Argument parsing for `md-to-docx`.
//!
//! Flag definitions come from `clap::Parser`. The error text matches
//! `src/cli.ts`, which the Jest CLI tests assert on.

use clap::error::{ContextKind, ContextValue};
use clap::{ArgAction, Parser};

pub const HELP_TEXT: &str = "\
Usage:
  md-to-docx <input.md|-> <output.docx|-> [options]
  md-to-docx --batch <input-directory> <output-directory> [options]
  md-to-docx <template.docx> <output.docx> --patches <patches.json>

Options:
  --options, -o <options.json>  Conversion options
  --reference <reference.docx> Adopt reference document styles
  --patches <patches.json>     Replace named placeholders with Markdown
  --base-dir <directory>      Base directory for local image paths
  --batch                     Convert .md files recursively, preserving directories
  --watch                     Rebuild when inputs, options, or local assets change
  --help, -h                  Show help

Use - for stdin/stdout. Diagnostics go to stderr when writing binary stdout.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub input: String,
    pub output: String,
    pub options: Option<String>,
    pub reference: Option<String>,
    pub patches: Option<String>,
    pub base_directory: Option<String>,
    pub batch: bool,
    pub watch: bool,
}

#[derive(Debug)]
pub enum Command {
    Help,
    Run(ParsedArgs),
}

#[derive(Parser, Debug)]
#[command(
    name = "md-to-docx",
    disable_help_flag = true,
    disable_version_flag = true
)]
struct ClapArgs {
    /// Show help.
    #[arg(short = 'h', long = "help", action = ArgAction::SetTrue)]
    help: bool,

    /// Convert `.md` files under the input directory.
    #[arg(long = "batch", action = ArgAction::SetTrue)]
    batch: bool,

    /// Rebuild when inputs change.
    #[arg(long = "watch", action = ArgAction::SetTrue)]
    watch: bool,

    /// Conversion options JSON.
    #[arg(short = 'o', long = "options", value_name = "options.json")]
    options: Option<String>,

    /// Reference `.docx` whose styles are adopted.
    #[arg(long = "reference", value_name = "reference.docx")]
    reference: Option<String>,

    /// Placeholder patch JSON.
    #[arg(long = "patches", value_name = "patches.json")]
    patches: Option<String>,

    /// Base directory for local image paths.
    #[arg(long = "base-dir", value_name = "directory")]
    base_dir: Option<String>,

    /// Markdown input path, or `-` for stdin.
    #[arg(value_name = "input.md|-", allow_hyphen_values = true)]
    input: Option<String>,

    /// Docx output path, or `-` for stdout.
    #[arg(value_name = "output.docx|-", allow_hyphen_values = true)]
    output: Option<String>,
}

pub fn parse_command(argv: &[String]) -> Result<Command, String> {
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        let _ = parse_with_clap(argv);
        return Ok(Command::Help);
    }
    let typescript = parse_like_typescript(argv);
    let clap = parse_with_clap(argv);
    if let Err(message) = typescript {
        return Err(message);
    }
    let clap_args = clap.map_err(|error| translate_clap(&error))?;
    if clap_args.help {
        return Ok(Command::Help);
    }
    let parsed = from_clap(clap_args)?;
    validate_combinations(&parsed)?;
    Ok(Command::Run(parsed))
}

fn parse_with_clap(argv: &[String]) -> Result<ClapArgs, clap::Error> {
    let mut full = Vec::with_capacity(argv.len() + 1);
    full.push("md-to-docx".to_string());
    full.extend(argv.iter().cloned());
    ClapArgs::try_parse_from(full)
}

fn from_clap(args: ClapArgs) -> Result<ParsedArgs, String> {
    let input = args.input.ok_or_else(expected_positionals)?;
    let output = args.output.ok_or_else(expected_positionals)?;
    Ok(ParsedArgs {
        input,
        output,
        options: args.options,
        reference: args.reference,
        patches: args.patches,
        base_directory: args.base_dir,
        batch: args.batch,
        watch: args.watch,
    })
}

fn expected_positionals() -> String {
    "Expected exactly 2 positional arguments: <input.md> <output.docx>".to_string()
}

/// Left-to-right scan from `parseCliArgs` in `src/cli.ts`.
fn parse_like_typescript(args: &[String]) -> Result<(), String> {
    let mut options = None;
    let mut reference = None;
    let mut patches = None;
    let mut base_directory = None;
    let mut batch = false;
    let mut watch = false;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--batch" {
            batch = true;
            index += 1;
            continue;
        }
        if arg == "--watch" {
            watch = true;
            index += 1;
            continue;
        }
        if matches!(
            arg.as_str(),
            "--options" | "-o" | "--reference" | "--patches" | "--base-dir"
        ) {
            let value = args.get(index + 1).filter(|value| !value.starts_with('-'));
            let Some(value) = value else {
                return Err(format!("Missing value for {arg}"));
            };
            match arg.as_str() {
                "--options" | "-o" => options = Some(value.clone()),
                "--reference" => reference = Some(value.clone()),
                "--patches" => patches = Some(value.clone()),
                "--base-dir" => base_directory = Some(value.clone()),
                _ => unreachable!("flag was matched above"),
            }
            index += 2;
            continue;
        }
        if arg.starts_with('-') && arg != "-" {
            return Err(format!("Unknown argument: {arg}"));
        }
        positional.push(arg.clone());
        index += 1;
    }
    if positional.len() != 2 {
        return Err(expected_positionals());
    }
    let _ = (options, reference, patches, base_directory, batch, watch);
    Ok(())
}

fn validate_combinations(parsed: &ParsedArgs) -> Result<(), String> {
    if parsed.patches.is_some()
        && (parsed.reference.is_some() || parsed.batch || parsed.input == "-")
    {
        return Err(
            "--patches cannot be combined with --reference, --batch, or stdin".to_string(),
        );
    }
    if (parsed.watch || parsed.batch) && (parsed.input == "-" || parsed.output == "-") {
        return Err("--watch and --batch require file paths".to_string());
    }
    if parsed.batch && resolve_path(&parsed.input) == resolve_path(&parsed.output) {
        return Err("Batch input and output directories must differ".to_string());
    }
    Ok(())
}

pub fn resolve_path(path: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    };
    let mut normalized = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn translate_clap(error: &clap::Error) -> String {
    let invalid = match error.get(ContextKind::InvalidArg) {
        Some(ContextValue::String(value)) => Some(value.clone()),
        Some(ContextValue::Strings(values)) => values.first().cloned(),
        _ => None,
    };
    match error.kind() {
        clap::error::ErrorKind::UnknownArgument => match invalid {
            Some(arg) if arg.starts_with('-') => format!("Unknown argument: {arg}"),
            Some(_) | None => expected_positionals(),
        },
        clap::error::ErrorKind::TooManyValues
        | clap::error::ErrorKind::TooFewValues
        | clap::error::ErrorKind::WrongNumberOfValues => expected_positionals(),
        clap::error::ErrorKind::MissingRequiredArgument | clap::error::ErrorKind::InvalidValue => {
            if let Some(flag) = missing_value_flag(error) {
                format!("Missing value for {flag}")
            } else {
                expected_positionals()
            }
        }
        clap::error::ErrorKind::DisplayHelp => HELP_TEXT.to_string(),
        _ => error.to_string(),
    }
}

fn missing_value_flag(error: &clap::Error) -> Option<String> {
    let raw = match error.get(ContextKind::InvalidArg) {
        Some(ContextValue::String(value)) => value.clone(),
        Some(ContextValue::Strings(values)) => values.first().cloned()?,
        _ => return None,
    };
    for flag in ["--options", "-o", "--reference", "--patches", "--base-dir"] {
        if raw.contains(flag) {
            return Some(flag.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{parse_command, Command};

    fn parse(args: &[&str]) -> Result<Command, String> {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        parse_command(&owned)
    }

    #[test]
    fn help_flags_win_over_other_arguments() {
        assert!(matches!(parse(&["--help"]).unwrap(), Command::Help));
        assert!(matches!(parse(&["-h"]).unwrap(), Command::Help));
        assert!(matches!(
            parse(&["a.md", "--help", "b.docx"]).unwrap(),
            Command::Help
        ));
    }

    #[test]
    fn rejects_unknown_and_missing_values() {
        let unknown = parse(&["a.md", "b.docx", "--verbose"]).unwrap_err();
        assert!(unknown.contains("Unknown argument: --verbose"), "{unknown}");
        let missing = parse(&["a.md", "b.docx", "--options"]).unwrap_err();
        assert!(missing.contains("Missing value for --options"), "{missing}");
        let extra = parse(&["a.md", "b.docx", "extra.md"]).unwrap_err();
        assert!(
            extra.contains("Expected exactly 2 positional arguments"),
            "{extra}"
        );
        let one = parse(&["a.md"]).unwrap_err();
        assert!(
            one.contains("Expected exactly 2 positional arguments"),
            "{one}"
        );
    }

    #[test]
    fn dash_is_stdin_and_stdout() {
        let Command::Run(parsed) = parse(&["-", "-"]).unwrap() else {
            panic!("expected a run");
        };
        assert_eq!(parsed.input, "-");
        assert_eq!(parsed.output, "-");
    }
}
