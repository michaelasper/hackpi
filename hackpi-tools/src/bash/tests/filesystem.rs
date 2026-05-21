use std::path::Path;

use super::new_session;
use super::output_stdout;
use super::BashSession;
use super::FileSystem;
use super::InMemoryFs;

// ── ls ─────────────────────────────────────────────────────────────

#[test]
fn test_ls_home() {
    let mut session = new_session();
    let out = session.execute("ls /home/user");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello.txt"));
}

#[test]
fn test_ls_bad_path() {
    let mut session = new_session();
    let out = session.execute("ls /nonexistent");
    assert_eq!(out.exit_code, 1);
}

// ── cat ────────────────────────────────────────────────────────────

#[test]
fn test_cat_file() {
    let mut session = new_session();
    let out = session.execute("cat hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello world"));
}

#[test]
fn test_cat_bad_file() {
    let mut session = new_session();
    let out = session.execute("cat nonexistent.txt");
    assert_eq!(out.exit_code, 1);
}

// ── mkdir ──────────────────────────────────────────────────────────

#[test]
fn test_mkdir() {
    let mut session = new_session();
    let out = session.execute("mkdir -p /tmp/newdir");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_dir(Path::new("/tmp/newdir")));
}

#[test]
fn test_mkdir_p() {
    let mut session = new_session();
    let out = session.execute("mkdir -p /tmp/a/b/c");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_dir(Path::new("/tmp/a/b/c")));
}

#[test]
fn test_mkdir_with_parent_traversal() {
    let mut session = new_session();
    let out = session.execute("mkdir -p /tmp/a/b/../c");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_dir(Path::new("/tmp/a/c")));
    assert!(!session.fs.is_dir(Path::new("/tmp/a/b")));
}

// ── touch ──────────────────────────────────────────────────────────

#[test]
fn test_touch_new_file() {
    let mut session = new_session();
    let out = session.execute("touch /tmp/newfile.txt");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_file(Path::new("/tmp/newfile.txt")));
}

// ── rm ─────────────────────────────────────────────────────────────

#[test]
fn test_rm_file() {
    let mut session = new_session();
    let out = session.execute("rm /home/user/hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/home/user/hello.txt")));
}

#[test]
fn test_rm_rf_dir() {
    let mut session = new_session();
    session.execute("mkdir -p /tmp/rmtest/subdir");
    let out = session.execute("rm -rf /tmp/rmtest");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/tmp/rmtest")));
}

#[test]
fn test_rm_dir_without_rf_fails() {
    let mut session = new_session();
    session.execute("mkdir /tmp/rmtest2");
    let out = session.execute("rm /tmp/rmtest2");
    assert_eq!(out.exit_code, 1);
}

#[test]
fn test_rm_with_parent_traversal() {
    let mut session = new_session();
    session.execute("mkdir -p /tmp/a");
    session
        .fs
        .write(Path::new("/tmp/b.txt"), b"content")
        .unwrap();
    let out = session.execute("rm /tmp/a/../b.txt");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/tmp/b.txt")));
}

// ── cp ─────────────────────────────────────────────────────────────

#[test]
fn test_cp_file() {
    let mut session = new_session();
    let out = session.execute("cp hello.txt /tmp/hello_copy.txt");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_file(Path::new("/tmp/hello_copy.txt")));
}

// ── mv ─────────────────────────────────────────────────────────────

#[test]
fn test_mv_file() {
    let mut session = new_session();
    let out = session.execute("mv hello.txt /tmp/moved.txt");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/home/user/hello.txt")));
    assert!(session.fs.is_file(Path::new("/tmp/moved.txt")));
}

// ── head / tail / wc / sort ────────────────────────────────────────

#[test]
fn test_head_default() {
    let mut session = new_session();
    let out = session.execute("head hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello world"));
}

#[test]
fn test_tail_default() {
    let mut session = new_session();
    let out = session.execute("tail hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("line three"));
}

#[test]
fn test_wc() {
    let mut session = new_session();
    let out = session.execute("wc hello.txt");
    assert_eq!(out.exit_code, 0);
    let parts: Vec<&str> = output_stdout(&out).split_whitespace().collect();
    assert_eq!(parts[0], "3");
}

#[test]
fn test_sort() {
    let mut session = new_session();
    let out = session.execute("sort numbers.txt");
    assert_eq!(out.exit_code, 0);
    let lines: Vec<&str> = output_stdout(&out).lines().collect();
    assert_eq!(lines, vec!["1", "2", "3"]);
}

// ── Parent traversal in redirects ──────────────────────────────────

#[test]
fn test_echo_with_parent_traversal() {
    let mut session = new_session();
    let out = session.execute("echo hello > /tmp/a/../b.txt");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_file(Path::new("/tmp/b.txt")));
    let content = session.fs.read(Path::new("/tmp/b.txt")).unwrap();
    assert_eq!(String::from_utf8_lossy(&content), "hello\n");
    assert!(!session.fs.is_dir(Path::new("/tmp/a")));
}

// ── ln (symlinks and hardlinks) ────────────────────────────────────

#[test]
fn test_ln_symlink_creates_link() {
    let mut session = new_session();
    session
        .fs
        .write(Path::new("/home/user/target.txt"), b"link target")
        .unwrap();
    let out = session.execute("ln -s target.txt link.txt");
    assert_eq!(out.exit_code, 0);
    let content = session.fs.read(Path::new("/home/user/link.txt")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&content),
        "link target",
        "symlink should resolve to target content"
    );
}

#[test]
fn test_ln_symlink_missing_target() {
    let mut session = new_session();
    let out = session.execute("ln -s nonexistent.txt link.txt");
    assert_eq!(
        out.exit_code, 0,
        "ln -s should succeed even with missing target"
    );
    // Reading a dangling symlink should fail
    let result = session.fs.read(Path::new("/home/user/link.txt"));
    assert!(result.is_err(), "reading a dangling symlink should fail");
}

#[test]
fn test_ln_hardlink() {
    let mut session = new_session();
    session
        .fs
        .write(Path::new("/home/user/original.txt"), b"hardlink content")
        .unwrap();
    let out = session.execute("ln original.txt hardlink.txt");
    assert_eq!(out.exit_code, 0);
    let content = session
        .fs
        .read(Path::new("/home/user/hardlink.txt"))
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&content), "hardlink content");
}

#[test]
fn test_ln_missing_operand() {
    let mut session = new_session();
    let out = session.execute("ln");
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing operand"));
}

// ── Session workdir ────────────────────────────────────────────────

#[test]
fn test_session_workdir_set_via_cwd() {
    let mut session = new_session();
    session.cwd = Path::new("/home").to_path_buf();
    assert_eq!(session.cwd, Path::new("/home"));

    // Test with relative path that goes to root
    let mut session = new_session();
    session.cwd = Path::new("/home").to_path_buf();
    assert_eq!(session.cwd, Path::new("/home"), "workdir should be /home");
}

// ── bashrc / InMemoryFs ────────────────────────────────────────────

#[test]
fn test_home_user_bashrc_exists() {
    let fs = Box::new(InMemoryFs::with_home(&std::path::PathBuf::from("/")));
    let session = BashSession::with_workspace(fs, std::path::PathBuf::from("/"));
    assert!(session.fs.is_file(Path::new("/home/user/.bashrc")));
}

#[test]
fn test_default_inmemoryfs_does_not_have_home() {
    // The default InMemoryFs should be minimal — no /home/user
    let fs = InMemoryFs::default();
    assert!(!fs.is_dir(std::path::Path::new("/home")));
}

#[test]
fn test_readlink_on_symlink() {
    let session = new_session();
    session
        .fs
        .write(Path::new("/home/user/real.txt"), b"content")
        .unwrap();
    session
        .fs
        .symlink(
            Path::new("/home/user/real.txt"),
            Path::new("/home/user/link.txt"),
        )
        .unwrap();
    let target = session
        .fs
        .read_link(Path::new("/home/user/link.txt"))
        .unwrap();
    assert_eq!(target, Path::new("/home/user/real.txt"));
}

// ── Concurrency ────────────────────────────────────────────────────

#[test]
fn test_concurrent_reads_do_not_deadlock() {
    let fs = Box::new(InMemoryFs::default());
    fs.write(Path::new("/file1.txt"), b"content1").unwrap();
    fs.write(Path::new("/file2.txt"), b"content2").unwrap();
    fs.write(Path::new("/file3.txt"), b"content3").unwrap();

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..10 {
            handles.push(s.spawn(|| {
                for _ in 0..100 {
                    let _r1 = fs.read(Path::new("/file1.txt"));
                    let _r2 = fs.read(Path::new("/file2.txt"));
                    let _r3 = fs.read(Path::new("/file3.txt"));
                    let _e = fs.exists(Path::new("/file1.txt"));
                    let _d = fs.is_dir(Path::new("/"));
                    let _f = fs.is_file(Path::new("/file1.txt"));
                }
            }));
        }
    });

    assert_eq!(fs.read(Path::new("/file1.txt")).unwrap(), b"content1");
    assert_eq!(fs.read(Path::new("/file2.txt")).unwrap(), b"content2");
}
