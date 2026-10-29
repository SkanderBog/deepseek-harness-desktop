use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::{ffi::OsStrExt, ffi::OsStringExt, process::CommandExt};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn print() -> Result<(), String> {
    let inherited = std::env::var_os("PATH");
    fix_path_env::fix().map_err(|error| error.to_string())?;
    let restored = std::env::var_os("PATH");
    let mut paths = Vec::new();
    for value in restored.iter().chain(inherited.iter()) {
        for path in std::env::split_paths(value) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    let path = std::env::join_paths(paths).map_err(|error| error.to_string())?;
    std::io::stdout()
        .write_all(path.as_bytes())
        .map_err(|error| error.to_string())
}

pub fn restore(executable: &std::path::Path) -> Result<(), String> {
    let mut output = tempfile::tempfile().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .arg("--print-login-path")
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|error| error.to_string())?)
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let error = match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => return Err(format!("login PATH helper exited with {status}")),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Ok(None) => "login PATH helper timed out after 3 seconds".to_owned(),
            Err(error) => format!("login PATH helper wait failed: {error}"),
        };
        let group_killed = unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) };
        let group_error = (group_killed != 0).then(std::io::Error::last_os_error);
        let _ = child.kill();
        child
            .wait()
            .map_err(|cleanup| format!("{error}; reap failed: {cleanup}"))?;
        if let Some(cleanup) = group_error {
            if cleanup.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("{error}; process group cleanup failed: {cleanup}"));
            }
        }
        return Err(error);
    }
    output
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 1_048_576 || bytes.contains(&0) || bytes.is_empty() {
        return Err("invalid login PATH helper output".into());
    }
    std::env::set_var("PATH", OsString::from_vec(bytes));
    Ok(())
}
