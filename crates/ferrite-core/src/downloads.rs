//! Pinned build inputs: immutable hashes, bounded archives, no executable from an unverified cache.
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};

static MANIFEST: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/downloads.json")).expect("pinned downloads")
});
static INDEX: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/build-index.json")).expect("pinned build index")
});
pub const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}
pub fn artifact(name: &str, url: &str) -> Result<Artifact> {
    let entry = &MANIFEST["entries"][url];
    let sha = entry["sha256"]
        .as_str()
        .context("Нет закреплённого SHA-256 для компонента")?;
    let size = entry["size"].as_u64().context("Нет размера компонента")?;
    if size == 0
        || size > MAX_ARCHIVE
        || sha.len() != 64
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("Некорректная запись зависимости {name}");
    }
    Ok(Artifact {
        name: name.into(),
        url: url.into(),
        sha256: sha.into(),
        size,
    })
}
pub fn product(name: &str) -> Result<Artifact> {
    let data = &INDEX[name]["versions"][0];
    for kind in ["release", "debug"] {
        if let Some(url) = data["links"][kind].as_str()
            && let Ok(a) = artifact(name, url)
        {
            return Ok(a);
        }
    }
    if name == "OpenCorePkg" {
        return artifact(
            name,
            "https://github.com/dortania/build-repo/releases/download/OpenCorePkg-963025f/OpenCore-1.0.9-DEBUG.zip",
        );
    }
    bail!(
        "{name}: выбранный релиз отсутствует в закреплённом manifest; обновите manifest, не заменяйте проверку хеша"
    )
}
pub fn client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent("Ferrite/0.2")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn fetch(
    a: &Artifact,
    cache: &Path,
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<PathBuf> {
    fs::create_dir_all(cache)?;
    let cache = fs::canonicalize(cache)?;
    let lock_path = cache.join(format!("{}.lock", a.sha256));
    if lock_path.is_symlink() {
        bail!("Cache lock is a symlink");
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let output = cache.join(&a.sha256);
    if output.exists() {
        if output.is_symlink() {
            bail!("Cache artifact is a symlink");
        }
        if fs::metadata(&output)?.len() == a.size && hash_file(&output)? == a.sha256 {
            return Ok(output);
        }
        bail!(
            "Повреждён кеш {}. Удалите этот файл и повторите загрузку: {}",
            a.name,
            output.display()
        );
    }
    let client = client()?;
    let sources = [
        a.url.clone(),
        format!("https://nullmothsystems.com/mirror/{}", a.sha256),
    ];
    let mut failures = vec![];
    for source in sources {
        let result = (|| -> Result<PathBuf> {
            let parsed = reqwest::Url::parse(&source)?;
            if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some()
            {
                bail!("Зависимости требуют HTTPS без credentials");
            }
            let mut response = client.get(&source).send()?.error_for_status()?;
            if response.content_length().is_some_and(|n| n != a.size) {
                bail!("Размер HTTP не соответствует manifest");
            }
            let mut temp = tempfile::NamedTempFile::new_in(&cache)?;
            let mut hasher = Sha256::new();
            let mut total = 0;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = response.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                total += count as u64;
                if total > a.size {
                    bail!("Файл больше закреплённого размера");
                }
                hasher.update(&buffer[..count]);
                temp.write_all(&buffer[..count])?;
                progress(&a.name, total, a.size);
            }
            if total != a.size || format!("{:x}", hasher.finalize()) != a.sha256 {
                bail!("SHA-256/размер {} не совпадают", a.name);
            }
            temp.as_file().sync_all()?;
            temp.persist_noclobber(&output).map_err(|e| e.error)?;
            Ok(output.clone())
        })();
        match result {
            Ok(path) => return Ok(path),
            Err(e) => failures.push(format!("{e:#}")),
        }
    }
    bail!("Не удалось загрузить {}: {}", a.name, failures.join("; "))
}
pub fn extract_zip(archive: &Path, destination: &Path, expected_sha: &str) -> Result<()> {
    if destination.exists() {
        bail!("Папка распаковки уже существует");
    }
    let file = File::open(archive)?;
    if file.metadata()?.len() > MAX_ARCHIVE {
        bail!("Архив превышает 512 MiB");
    }
    let mut archive_bytes = vec![];
    file.take(MAX_ARCHIVE + 1).read_to_end(&mut archive_bytes)?;
    if archive_bytes.len() as u64 > MAX_ARCHIVE
        || format!("{:x}", Sha256::digest(&archive_bytes)) != expected_sha
    {
        bail!("Архив изменился перед распаковкой");
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes))?;
    if zip.len() > 20000 {
        bail!("Слишком много файлов в архиве");
    }
    // Validate all members before creating any output files.
    let mut total = 0u64;
    let mut names = std::collections::BTreeSet::new();
    for i in 0..zip.len() {
        let e = zip.by_index(i)?;
        let path = e
            .enclosed_name()
            .context("ZIP содержит выход за пределы папки")?;
        if e.name().contains('\\')
            || e.name().contains(':')
            || e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            bail!("ZIP содержит ссылку/некорректный путь");
        }
        let norm = path.to_string_lossy().to_lowercase();
        if !names.insert(norm) {
            bail!("ZIP содержит повторяющийся путь");
        }
        total = total
            .checked_add(e.size())
            .context("Размер ZIP переполнен")?;
        if total > 1024 * 1024 * 1024 {
            bail!("ZIP распаковывается более чем в 1 GiB");
        }
    }
    fs::create_dir_all(destination)?;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i)?;
        let output = destination.join(e.enclosed_name().context("ZIP path")?);
        if e.is_dir() {
            fs::create_dir_all(&output)?;
        } else {
            if let Some(p) = output.parent() {
                fs::create_dir_all(p)?;
            }
            let mut target = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&output)?;
            let written = std::io::copy(&mut e, &mut target)?;
            if written > MAX_ARCHIVE {
                bail!("Распакованный файл слишком велик");
            }
        }
    }
    Ok(())
}
pub fn files(root: &Path, suffix: &str) -> Result<Vec<PathBuf>> {
    let mut result = vec![];
    walk(root, suffix, 0, &mut result)?;
    Ok(result)
}
fn walk(root: &Path, suffix: &str, depth: usize, result: &mut Vec<PathBuf>) -> Result<()> {
    if depth > 32 || result.len() > 20000 {
        bail!("Слишком глубокая/большая папка компонентов");
    }
    for entry in fs::read_dir(root)? {
        let e = entry?;
        let kind = e.file_type()?;
        let p = e.path();
        if kind.is_symlink() {
            bail!("Ссылка в папке компонентов");
        }
        if p.file_name()
            .is_some_and(|s| s.to_string_lossy().ends_with(suffix))
        {
            result.push(p.clone());
        }
        if kind.is_dir() {
            walk(&p, suffix, depth + 1, result)?;
        }
    }
    Ok(())
}
pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if source.is_symlink() {
        bail!("Ссылка в исходной папке");
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let e = entry?;
        let kind = e.file_type()?;
        let to = destination.join(e.file_name());
        if kind.is_symlink() {
            bail!("Ссылка в дереве файлов");
        }
        if kind.is_dir() {
            copy_tree(&e.path(), &to)?;
        } else if kind.is_file() {
            if to.exists() {
                bail!("Файл назначения уже существует: {}", to.display());
            }
            fs::copy(e.path(), to)?;
        } else {
            bail!("Специальный файл в дереве");
        }
    }
    Ok(())
}
pub fn snapshot_artifact(source: &Path, artifact: &Artifact, destination: &Path) -> Result<()> {
    let mut bytes = vec![];
    File::open(source)?
        .take(artifact.size + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != artifact.size
        || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
    {
        bail!("Архив изменился перед сохранением");
    }
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    out.write_all(&bytes)?;
    out.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_unpinned_product() {
        assert!(product("MadeUpKext").is_err());
    }
    #[test]
    fn opencore_is_pinned() {
        assert_eq!(product("OpenCorePkg").unwrap().sha256.len(), 64);
    }
    #[test]
    fn corrupt_cache_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let a = Artifact {
            name: "test".into(),
            url: "https://example.com".into(),
            sha256: "0".repeat(64),
            size: 3,
        };
        fs::write(dir.path().join(&a.sha256), b"bad").unwrap();
        assert!(fetch(&a, dir.path(), &mut |_, _, _| {}).is_err());
    }
}
