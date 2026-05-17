//! `ori-docsite` CLI entry point.
//!
//! Usage: `ori-docsite --input <dir> --output <dir>`.
//!
//! Emits a deterministic static documentation site rendered from the markdown
//! files under `--input`. On success prints a JSON `SiteReport` to stdout so
//! the output is consumable by CI scripts.

use std::path::PathBuf;
use std::process::ExitCode;

use ori_docsite::build_site;

const USAGE: &str = "\
ori-docsite --input <dir> --output <dir>

Generate a static HTML documentation site from a directory of markdown files.

Options:
  --input  <dir>   Source directory to scan recursively for `*.md` files.
  --output <dir>   Output directory for generated HTML and `style.css`.
  -h, --help       Show this help and exit.
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let parsed = match parse_args(&argv[1..]) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("ori-docsite: {msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    if parsed.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let input = match parsed.input {
        Some(p) => p,
        None => {
            eprintln!("ori-docsite: missing --input <dir>\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let output = match parsed.output {
        Some(p) => p,
        None => {
            eprintln!("ori-docsite: missing --output <dir>\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match build_site(&input, &output) {
        Ok(report) => {
            match serde_json::to_string_pretty(&report) {
                Ok(json) => println!("{json}"),
                Err(e) => {
                    eprintln!("ori-docsite: failed to serialize report: {e}");
                    return ExitCode::from(1);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ori-docsite: {e}");
            ExitCode::from(1)
        }
    }
}

struct ParsedArgs {
    help: bool,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut parsed = ParsedArgs {
        help: false,
        input: None,
        output: None,
    };
    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                parsed.help = true;
                i += 1;
            }
            "--input" => {
                let next = args.get(i + 1).ok_or_else(|| {
                    "expected value after --input".to_string()
                })?;
                parsed.input = Some(PathBuf::from(next));
                i += 2;
            }
            "--output" => {
                let next = args.get(i + 1).ok_or_else(|| {
                    "expected value after --output".to_string()
                })?;
                parsed.output = Some(PathBuf::from(next));
                i += 2;
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
    }
    Ok(parsed)
}
