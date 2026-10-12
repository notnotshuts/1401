use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn default_workspace() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Ferrite")
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Library/Application Support/Ferrite")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join(".local/share")
            })
            .join("ferrite")
    }
}
/// Never overwrite an existing file. Finished JSON appears with one rename inside a new directory.
pub fn save_new<T: Serialize>(root: &Path, name: &str, data: &T) -> Result<PathBuf> {
    if Path::new(name).components().count() != 1 {
        bail!("Некорректное имя файла");
    }
    let dir = new_session(root)?;
    let destination = dir.join(name);
    let temp = dir.join(".partial");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec_pretty(data)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temp, &destination)?;
    Ok(destination)
}
/// Bound process duration and total stdout/stderr, drain both pipes concurrently to prevent deadlocks.
pub fn run_bounded(program: &str, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Не удалось запустить {program}"))?;
    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;
    let out = thread::spawn(move || {
        let mut v = Vec::new();
        stdout
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut v)
            .map(|_| v)
    });
    let err = thread::spawn(move || {
        let mut v = Vec::new();
        stderr
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut v)
            .map(|_| v)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{program}: превышено время ожидания");
        }
        thread::sleep(Duration::from_millis(25));
    };
    let output = out.join().map_err(|_| anyhow::anyhow!("stdout thread"))??;
    let error = err.join().map_err(|_| anyhow::anyhow!("stderr thread"))??;
    if output.len() > 16 * 1024 * 1024 || error.len() > 16 * 1024 * 1024 {
        bail!("Вывод {program} превышает допустимый размер");
    }
    if !status.success() {
        bail!(
            "{program}: exit {status}; {} {}",
            String::from_utf8_lossy(&error[..error.len().min(2000)]),
            String::from_utf8_lossy(&output[..output.len().min(6000)])
        );
    }
    Ok(output)
}

pub fn new_session(root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(root)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let dir = root.join(format!("session-{stamp}-{}", std::process::id()));
    fs::create_dir(&dir)?;
    Ok(dir)
}
