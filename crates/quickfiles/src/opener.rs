//! `[[opener]]` blocks of `config.toml`: open some file types with a program of the user's
//! choice instead of the Windows default.
//!
//! ```toml
//! [[opener]]
//! extensions = ["nu", "nuon"]     # with or without the dot, any case
//! program = 'C:\Tools\editor.exe' # `path` works too; single quotes keep the backslashes
//! args = ["-o", "$1"]             # optional; default is just the file path
//! ```
//!
//! `$1` is replaced by the file's full path inside each argument, so paths with spaces stay one
//! argument. No shell is involved.

use std::{io, path::Path, process::Command};

use toml::Value;

use crate::config::Block;

#[derive(Debug, Clone, PartialEq)]
pub struct Opener {
    /// Lower case, without the dot.
    exts: Vec<String>,
    program: String,
    args: Vec<String>,
}

/// Reads every block; bad ones are skipped and reported.
pub fn from_blocks(blocks: &[Block]) -> (Vec<Opener>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for b in blocks {
        for key in b.table.keys() {
            if !matches!(
                key.as_str(),
                "extension" | "extensions" | "program" | "path" | "args"
            ) {
                problems.push(format!(
                    "config line {}: unknown opener setting \"{key}\"",
                    b.line
                ));
            }
        }
        let get = |a: &str, b2: &str| b.table.get(a).or_else(|| b.table.get(b2));
        let exts: Vec<String> = get("extensions", "extension")
            .and_then(|v| strings(v, |s| s.split([',', ' ']).map(str::to_owned).collect()))
            .unwrap_or_default()
            .into_iter()
            .map(|x| x.trim().trim_start_matches(['.', '*']).to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
        let program = get("program", "path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty());
        let args = b
            .table
            .get("args")
            .and_then(|v| strings(v, |s| s.split_whitespace().map(str::to_owned).collect()));
        match program {
            Some(program) if !exts.is_empty() => out.push(Opener {
                exts,
                program: program.to_owned(),
                args: args.unwrap_or_else(|| vec!["$1".to_owned()]),
            }),
            _ => problems.push(format!(
                "config line {}: [[opener]] needs extensions (a list) and a program",
                b.line
            )),
        }
    }
    (out, problems)
}

/// A TOML list of strings, or a single string turned into a list by `split`.
fn strings(value: &Value, split: fn(&str) -> Vec<String>) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Value::String(s) => Some(split(s)),
        _ => None,
    }
}

/// The first opener that lists this file's extension.
pub fn find<'a>(openers: &'a [Opener], path: &Path) -> Option<&'a Opener> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    openers.iter().find(|o| o.exts.iter().any(|e| *e == ext))
}

impl Opener {
    /// Starts the program and returns at once.
    pub fn launch(&self, path: &Path) -> io::Result<()> {
        let file = path.to_string_lossy();
        let mut cmd = Command::new(&self.program);
        cmd.args(self.args.iter().map(|a| a.replace("$1", &file)));
        if let Some(dir) = path.parent().filter(|d| d.is_dir()) {
            cmd.current_dir(dir);
        }
        cmd.spawn().map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(toml: &str) -> Block {
        Block {
            line: 1,
            table: toml.parse().unwrap(),
        }
    }

    #[test]
    fn matching_and_defaults() {
        let (o, p) = from_blocks(&[
            block("extension = \".NU\"\npath = 'ed.exe'\nargs = [\"-o\", \"$1\"]"),
            block("extensions = \"md, txt\"\nprogram = 'my ed.exe'"),
        ]);
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(find(&o, Path::new(r"C:\x\a.nu")).unwrap().program, "ed.exe");
        assert_eq!(
            find(&o, Path::new(r"C:\x\a.nu")).unwrap().args,
            ["-o", "$1"]
        );
        assert_eq!(find(&o, Path::new("b.TXT")).unwrap().program, "my ed.exe");
        assert_eq!(find(&o, Path::new("b.txt")).unwrap().args, ["$1"]);
        assert!(find(&o, Path::new("c.rs")).is_none());
        assert!(find(&o, Path::new("noext")).is_none());
    }

    #[test]
    fn bad_blocks_are_reported() {
        let (o, p) = from_blocks(&[
            block("extensions = [\"nu\"]"),
            block("bogus = 1"),
            block("extensions = [1]\nprogram = 'x'"),
        ]);
        assert!(o.is_empty());
        assert_eq!(p.len(), 4, "{p:?}");
    }
}
