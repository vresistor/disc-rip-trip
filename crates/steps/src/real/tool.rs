//! Running the bundled tools.
//!
//! A tool is found as `DRT_<NAME>` if that's set (a whole command line, so
//! development on the host can say `flatpak run --command=cyanrip
//! io.github.vresistor.disc-rip-trip`), and otherwise by name on the PATH, which in the
//! flatpak is where the bundled ones are.
//!
//! Output is read through a pseudo-terminal. cyanrip writes through stdio,
//! which buffers everything when it's a pipe: measured on a real disc, every
//! line of a two-track rip arrived at once when the process ended, so a
//! pipe gives no progress at all.

use anyhow::{anyhow, bail, Result};
use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use tokio::sync::mpsc;

/// The tools the built-in recipes run that the flatpak bundles. MakeMKV
/// isn't one: its card says how to get it.
pub const BUNDLED: [&str; 4] = ["cyanrip", "redumper", "ffmpeg", "ffprobe"];

/// The command line a tool runs as: `DRT_<NAME>` if set, else its name.
fn line(name: &str) -> String {
    std::env::var(format!("DRT_{}", name.to_uppercase().replace('-', "_"))).unwrap_or_else(|_| name.to_owned())
}

/// Which of `names` can't be found: as a path if the command has a `/`,
/// else on the PATH.
pub fn missing(names: &[&'static str]) -> Vec<&'static str> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = |name: &str| {
        let line = line(name);
        let program = line.split_whitespace().next().unwrap_or(name);
        let runnable = |p: &Path| p.metadata().is_ok_and(|m| m.is_file() && std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0);
        if program.contains('/') {
            runnable(Path::new(program))
        } else {
            std::env::split_paths(&path).any(|dir| runnable(&dir.join(program)))
        }
    };
    names.iter().copied().filter(|n| !found(n)).collect()
}

/// A path as a tool's argument: one that starts with `-` is given as
/// `./-…`, so a file name can never be read as an option.
pub fn path_arg(p: &Path) -> std::ffi::OsString {
    if p.is_relative() && p.as_os_str().as_encoded_bytes().starts_with(b"-") {
        Path::new(".").join(p).into_os_string()
    } else {
        p.as_os_str().to_owned()
    }
}

pub fn command(name: &str) -> tokio::process::Command {
    let line = line(name);
    let mut parts = line.split_whitespace();
    let mut cmd = tokio::process::Command::new(parts.next().unwrap_or(name));
    cmd.args(parts);
    cmd.kill_on_drop(true);
    cmd
}

/// Runs to the end, handing each line (split at `\r` as well as `\n`, so
/// progress lines count) to `on_line`. Gives everything it printed and how it
/// exited. Stops the process if `stop` resolves first.
pub async fn run(mut cmd: tokio::process::Command, cwd: &Path, stop: impl std::future::Future<Output = ()>, mut on_line: impl FnMut(&str)) -> Result<(String, ExitStatus)> {
    let (master, slave) = pty()?;
    cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    let mut child = cmd.spawn().map_err(|e| anyhow!("couldn't start {:?}: {e}", cmd.as_std().get_program()))?;
    // Our copies of the terminal's child end must be closed by now (they
    // were moved into the Command, which spawn has dropped), or reading the
    // master never sees the end.
    drop(cmd);

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let mut f = std::fs::File::from(master);
        let mut buf = [0u8; 4096];
        let mut line = Vec::new();
        // EIO once the child has exited and closed its end: that's the end of output.
        while let Ok(n) = f.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    if !line.is_empty() {
                        let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
                        line.clear();
                    }
                } else {
                    line.push(b);
                }
            }
        }
        if !line.is_empty() {
            let _ = tx.send(String::from_utf8_lossy(&line).into_owned());
        }
    });

    let mut transcript = String::new();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(l) => {
                    // Kept in the step's record, as it printed it.
                    drt_engine::raw_here(&l);
                    on_line(&l);
                    transcript.push_str(&l);
                    transcript.push('\n');
                }
                None => break,
            },
            _ = &mut stop => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                bail!("stopped");
            }
        }
    }
    let status = child.wait().await?;
    Ok((transcript, status))
}

/// A pseudo-terminal, wide enough that nothing wraps.
fn pty() -> Result<(OwnedFd, OwnedFd)> {
    let (mut m, mut s) = (0, 0);
    let ws = libc::winsize { ws_row: 50, ws_col: 400, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: two out-parameters, no name or termios, and a winsize that
    // outlives the call.
    let r = unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut(), &ws) };
    if r != 0 {
        bail!("couldn't open a pseudo-terminal: {}", std::io::Error::last_os_error());
    }
    // SAFETY: openpty succeeded, so both are open descriptors that nothing
    // else owns.
    let (m, s) = unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
    // openpty doesn't set close-on-exec, so without this every tool started
    // while one runs would inherit its terminal, and could read its output.
    // The tool's own end is put on its stdout and stderr by dup2, which
    // clears the flag there.
    for fd in [&m, &s] {
        // SAFETY: F_SETFD on a descriptor we own; no pointers.
        if unsafe { libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(fd), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            bail!("couldn't set up a pseudo-terminal: {}", std::io::Error::last_os_error());
        }
    }
    Ok((m, s))
}

/// The last line worth showing someone, skipping the loader's complaints about
/// libcurl and cd-paranoia's warnings, which are never the cause.
pub fn last_meaningful_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.contains("no version information available") && !l.starts_with("++ WARN") && !l.contains("Checking /dev/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tool sees its own terminal on stdout and stderr and nothing else:
    /// not the app's end of it, nor another tool's.
    #[tokio::test]
    async fn a_tool_inherits_no_terminal_but_its_own() {
        let _other = pty().unwrap();
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "ls -l /proc/$$/fd"]);
        let (text, status) = run(cmd, Path::new("/"), std::future::pending(), |_| {}).await.unwrap();
        assert!(status.success(), "{text}");
        assert!(!text.contains("ptmx"), "{text}");
        assert_eq!(text.matches("/dev/pts/").count(), 2, "stdout and stderr: {text}");
    }

    #[test]
    fn a_path_is_never_an_option() {
        assert_eq!(path_arg(Path::new("-y.flac")), "./-y.flac");
        assert_eq!(path_arg(Path::new("--help/01.flac")), "./--help/01.flac");
        assert_eq!(path_arg(Path::new("/music/-y.flac")), "/music/-y.flac");
        assert_eq!(path_arg(Path::new("a/-y.flac")), "a/-y.flac");
    }

    #[test]
    fn a_tool_is_missing_only_when_neither_path_nor_its_variable_finds_it() {
        assert_eq!(missing(&["sh", "drt-no-such-tool"]), ["drt-no-such-tool"]);
        // A whole command line: only its first word has to exist.
        std::env::set_var("DRT_DRT_STAND_IN", "/bin/sh -c true");
        std::env::set_var("DRT_DRT_GONE", "/no/such/dir/tool --flag");
        assert_eq!(missing(&["drt-stand-in", "drt-gone"]), ["drt-gone"]);
    }
}
