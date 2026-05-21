use std::path::Path;

use super::filesystem::{FileSystem, InMemoryFs};
use super::session::{BashOutput, BashSession};

pub(super) mod builtins;
pub(super) mod errors;
pub(super) mod filesystem;
pub(super) mod pipeline;
pub(super) mod variables;

// Re-export types from parent modules so sub-modules can use super::TypeName
pub(super) use super::parser::{parse, tokenize, AstNode, RedirectOp};
pub(super) use super::tool::BashTool;

/// Create a new BashSession with test fixtures (hello.txt, numbers.txt, colors.txt)
pub(super) fn new_session() -> BashSession {
    let home = Path::new("/home/user");
    let fs = Box::new(InMemoryFs::with_home(&std::path::PathBuf::from("/")));
    let session = BashSession::with_workspace(fs, home.to_path_buf());

    session
        .fs
        .write(
            &home.join("hello.txt"),
            b"hello world\nline two\nline three\n",
        )
        .unwrap();
    session
        .fs
        .write(&home.join("numbers.txt"), b"3\n1\n2\n")
        .unwrap();
    session
        .fs
        .write(&home.join("colors.txt"), b"red\ngreen\nblue\n")
        .unwrap();

    session
}

/// Get trimmed stdout from a BashOutput
pub(super) fn output_stdout(out: &BashOutput) -> &str {
    out.stdout.trim()
}
