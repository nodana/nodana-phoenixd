use std::os::unix::process::CommandExt;
use std::{path::Path, process::Command};

fn command(root: &Path, args: impl Iterator<Item = std::ffi::OsString>) -> Command {
    let mut command = Command::new(root.join("lib/ld.so"));
    command
        .arg("--inhibit-cache")
        .arg("--library-path")
        .arg(root.join("lib"))
        .arg(root.join("libexec/phoenixd"))
        .args(args);
    for name in ["LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT"] {
        command.env_remove(name);
    }
    command
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args == ["--nodana-version"] {
        println!("{}", option_env!("NODANA_RELEASE").unwrap_or("development"));
        return;
    }
    let result = (|| -> std::io::Result<()> {
        let executable = std::env::current_exe()?;
        let root = executable
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| std::io::Error::other("missing bundle root"))?;
        Err(command(root, args.into_iter()).exec())
    })();
    if result.is_err() {
        eprintln!("NODANA_PHOENIXD_EXEC_FAILED");
        std::process::exit(127);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwards_arguments_and_removes_loader_overrides() {
        let c = command(
            Path::new("/bundle"),
            ["--datadir", "/wallet with spaces"]
                .into_iter()
                .map(Into::into),
        );
        assert_eq!(c.get_program(), "/bundle/lib/ld.so");
        assert_eq!(
            c.get_args().collect::<Vec<_>>(),
            [
                "--inhibit-cache",
                "--library-path",
                "/bundle/lib",
                "/bundle/libexec/phoenixd",
                "--datadir",
                "/wallet with spaces"
            ]
        );
        assert_eq!(c.get_envs().filter(|(_, v)| v.is_none()).count(), 3);
    }
}
