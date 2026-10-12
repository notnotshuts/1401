//! Apple Recovery protocol and RSA-signed chunklist verification, entirely in Rust.
use anyhow::{Context, Result, bail};
use rand::Rng;
use reqwest::{
    blocking::{Client, Response},
    header,
};
use rsa::{BigUint, Pkcs1v15Sign, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const KEY: &str = concat!(
    "C3E748CAD9CD384329E10E25A91E43E1A762FF529ADE578C935BDDF9B13F2179D4855E6FC89E9E29CA12517D17DFA1EDCE0BEBF0EA7B461FFE",
    "61D94E2BDF72C196F89ACD3536B644064014DAE25A15DB6BB0852ECBD120916318D1CCDEA3C84C92ED743FC176D0BACA920D3FCF3158AFF731",
    "F88CE0623182A8ED67E650515F75745909F07D415F55FC15A35654D118C55A462D37A3ACDA08612F3F3F6571761EFCCBCC299AEE99B3A4FD62",
    "12CCFFF5EF37A2C334E871191F7E1C31960E010A54E86FA3F62E6D6905E1CD57732410A3EB0C6B4DEFDABE9F59BF1618758C751CD56CEF851D",
    "1C0EAA1C558E37AC108DA9089863D20E2E7E4BF475EC66FE6B3EFDCF"
);
#[derive(Clone, Debug, Serialize)]
pub struct Info {
    pub name: String,
    pub board: String,
    pub product: String,
    pub image_url: String,
    pub chunklist_url: String,
    #[serde(skip_serializing)]
    image_token: String,
    #[serde(skip_serializing)]
    chunklist_token: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunk {
    pub size: u32,
    pub sha256: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recovery {
    pub image: PathBuf,
    pub chunklist: PathBuf,
    pub bytes: u64,
    pub chunks: usize,
    pub image_sha256: String,
    pub chunklist_sha256: String,
    pub board: String,
    pub product: String,
}
fn board(target: u8) -> Result<&'static str> {
    match target {
        25 => Ok("Mac-CFF7D910A743CAAF"),
        24 => Ok("Mac-7BA5B2D9E42DDD94"),
        23 => Ok("Mac-827FAC58A8FDFA22"),
        22 => Ok("Mac-4B682C642B45593E"),
        21 => Ok("Mac-FFE5EF870D7BA81A"),
        20 => Ok("Mac-42FD25EABCABB274"),
        19 => Ok("Mac-00BE6ED71E35EB86"),
        _ => bail!("Неизвестный Recovery target"),
    }
}
fn hex(n: usize) -> String {
    const SYMBOLS: &[u8] = b"0123456789ABCDEF";
    let mut rng = rand::rngs::OsRng;
    (0..n)
        .map(|_| SYMBOLS[rng.gen_range(0..16)] as char)
        .collect()
}
fn apple_host(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|h| {
        h == "apple.com" || h.ends_with(".apple.com") || h.ends_with(".cdn-apple.com")
    }) && url.username().is_empty()
        && url.password().is_none()
}
fn client() -> Result<Client> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .user_agent("InternetRecovery/1.0")
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 5
                || !apple_host(a.url())
                || !matches!(a.url().scheme(), "https" | "http")
            {
                a.stop()
            } else {
                a.follow()
            }
        }))
        .build()?)
}
fn session(client: &Client) -> Result<String> {
    let response = client
        .get("https://osrecovery.apple.com/")
        .send()?
        .error_for_status()?;
    for cookie in response.headers().get_all(header::SET_COOKIE) {
        for c in cookie.to_str()?.split(';') {
            let c = c.trim();
            if c.starts_with("session=") && !c.contains(['\r', '\n']) {
                return Ok(c.into());
            }
        }
    }
    bail!("Apple не вернул Recovery session cookie")
}
pub fn info(target: u8) -> Result<Info> {
    let board = board(target)?;
    let client = client()?;
    for attempt in 0..2 {
        let session = session(&client)?;
        let body = format!(
            "cid={}\nsn=00000000000000000\nbid={board}\nk={}\nfg={}\nos=latest",
            hex(16),
            hex(64),
            hex(64)
        );
        let response = client
            .post("https://osrecovery.apple.com/InstallationPayload/RecoveryImage")
            .header(header::COOKIE, session)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(body)
            .send()?;
        if matches!(response.status().as_u16(), 401 | 403) && attempt == 0 {
            continue;
        }
        let mut response = response
            .error_for_status()
            .context("Apple Recovery metadata: отказ сервера")?;
        let mut raw = String::new();
        response.by_ref().take(65537).read_to_string(&mut raw)?;
        if raw.len() > 65536 {
            bail!("Apple metadata слишком велико");
        }
        let map: std::collections::BTreeMap<_, _> =
            raw.lines().filter_map(|l| l.split_once(": ")).collect();
        let get = |k: &str| -> Result<String> {
            Ok(map
                .get(k)
                .with_context(|| format!("Apple metadata без {k}"))?
                .to_string())
        };
        let i = Info {
            name: crate::compatibility::version_name(target).into(),
            board: board.into(),
            product: get("AP")?,
            image_url: get("AU")?,
            chunklist_url: get("CU")?,
            image_token: get("AT")?,
            chunklist_token: get("CT")?,
        };
        for url in [&i.image_url, &i.chunklist_url] {
            let parsed = reqwest::Url::parse(url)?;
            if !apple_host(&parsed) || !matches!(parsed.scheme(), "http" | "https") {
                bail!("Apple Recovery вернул неподдерживаемый asset host");
            }
        }
        return Ok(i);
    }
    bail!("Apple отказал в Recovery session после обновления")
}
fn key() -> Result<RsaPublicKey> {
    Ok(RsaPublicKey::new(
        BigUint::from_bytes_be(&crate::config::bytes(KEY)?),
        BigUint::from(65537u32),
    )?)
}
pub fn parse_chunklist(raw: &[u8]) -> Result<Vec<Chunk>> {
    parse_with_key(raw, &key()?)
}
fn parse_with_key(raw: &[u8], key: &RsaPublicKey) -> Result<Vec<Chunk>> {
    if raw.len() < 36 || raw.len() > 4 * 1024 * 1024 {
        bail!("Chunklist size invalid");
    }
    let u32_at =
        |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().expect("bounded header"));
    let u64_at =
        |at: usize| u64::from_le_bytes(raw[at..at + 8].try_into().expect("bounded header"));
    let count = u64_at(12);
    let coff = u64_at(20);
    let soff = u64_at(28);
    if &raw[..4] != b"CNKL"
        || u32_at(4) != 36
        || raw[8] != 1
        || raw[9] != 1
        || raw[10] != 1
        || count == 0
        || count > 100000
        || coff != 36
        || soff != 36 + 36 * count
        || soff + 256 != raw.len() as u64
    {
        bail!("Неизвестная/неподписанная структура chunklist");
    }
    let mut signature = raw[soff as usize..].to_vec();
    signature.reverse();
    key.verify(
        Pkcs1v15Sign::new::<Sha256>(),
        &Sha256::digest(&raw[..soff as usize]),
        &signature,
    )
    .context("Подпись chunklist не принадлежит доверенному Apple EFI ROM key")?;
    let mut chunks = vec![];
    let mut total = 0u64;
    for i in 0..count as usize {
        let offset = 36 + i * 36;
        let size = u32_at(offset);
        if size == 0 || size > 128 * 1024 * 1024 {
            bail!("Chunk size вне допустимого диапазона");
        }
        total += size as u64;
        if total > 16 * 1024 * 1024 * 1024 {
            bail!("Recovery больше 16 GiB");
        }
        chunks.push(Chunk {
            size,
            sha256: raw[offset + 4..offset + 36].try_into()?,
        });
    }
    Ok(chunks)
}
fn asset(client: &Client, url: &str, token: &str, offset: Option<u64>) -> Result<Response> {
    if token.contains(['\r', '\n', ';']) {
        bail!("Invalid Apple asset token");
    }
    let mut req = client
        .get(url)
        .header(header::COOKIE, format!("AssetToken={token}"));
    if let Some(offset) = offset {
        req = req.header(header::RANGE, format!("bytes={offset}-"));
    }
    let response = req.send()?.error_for_status()?;
    if let Some(offset) = offset
        && (response.status().as_u16() != 206
            || !response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with(&format!("bytes {offset}-"))))
    {
        bail!("Apple не подтвердил resume offset {offset}");
    }
    Ok(response)
}
pub fn download(
    info: &Info,
    destination: &Path,
    progress: &mut impl FnMut(&str, u64, u64),
) -> Result<Recovery> {
    fs::create_dir_all(destination)?;
    let client = client()?;
    let mut response = asset(&client, &info.chunklist_url, &info.chunklist_token, None)?;
    let mut signed = vec![];
    response
        .by_ref()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut signed)?;
    let chunks = parse_chunklist(&signed)?;
    let total = chunks.iter().map(|c| c.size as u64).sum();
    let image = destination.join("BaseSystem.dmg");
    let chunklist = destination.join("BaseSystem.chunklist");
    if image.exists() || chunklist.exists() {
        bail!("Recovery не перезаписывает существующие файлы; выберите новую папку");
    }
    let partial = destination.join("BaseSystem.dmg.part");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let mut done = 0;
    let mut retries = 0;
    let mut body = asset(&client, &info.image_url, &info.image_token, None)?;
    let mut full_hash = Sha256::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let mut bytes = vec![0u8; chunk.size as usize];
        loop {
            match body.read_exact(&mut bytes) {
                Ok(()) => break,
                Err(e) => {
                    retries += 1;
                    if retries > 8 {
                        bail!("Recovery stream остановился после 8 resume: {e}");
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    body = asset(&client, &info.image_url, &info.image_token, Some(done))?;
                }
            }
        }
        if Sha256::digest(&bytes).as_slice() != chunk.sha256 {
            bail!(
                "Recovery chunk {}/{} не соответствует подписи Apple",
                i + 1,
                chunks.len()
            );
        }
        file.write_all(&bytes)?;
        full_hash.update(&bytes);
        done += chunk.size as u64;
        progress("Apple Recovery", done, total);
    }
    let mut extra = [0u8; 1];
    if body.read(&mut extra)? != 0 {
        bail!("Recovery больше подписанного списка");
    }
    file.sync_all()?;
    let cl_partial = destination.join("BaseSystem.chunklist.part");
    let mut cl_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cl_partial)?;
    cl_file.write_all(&signed)?;
    cl_file.sync_all()?;
    fs::rename(&partial, &image)?;
    fs::rename(cl_partial, &chunklist)?;
    Ok(Recovery {
        image,
        chunklist,
        bytes: done,
        chunks: chunks.len(),
        image_sha256: format!("{:x}", full_hash.finalize()),
        chunklist_sha256: format!("{:x}", Sha256::digest(&signed)),
        board: info.board.clone(),
        product: info.product.clone(),
    })
}
pub fn verify(image: &Path, chunklist: &Path) -> Result<u64> {
    if image.is_symlink() || chunklist.is_symlink() {
        bail!("Recovery file is a symlink");
    }
    let cl = fs::read(chunklist)?;
    let chunks = parse_chunklist(&cl)?;
    let mut image = std::fs::File::open(image)?;
    let mut total = 0;
    for (i, c) in chunks.iter().enumerate() {
        let mut bytes = vec![0u8; c.size as usize];
        image.read_exact(&mut bytes)?;
        if Sha256::digest(&bytes).as_slice() != c.sha256 {
            bail!("Recovery chunk {} повреждён", i + 1);
        }
        total += c.size as u64;
    }
    if image.read(&mut [0u8; 1])? != 0 {
        bail!("Recovery contains extra data");
    }
    Ok(total)
}
#[cfg(test)]
mod tests {
    use super::*;
    use rsa::RsaPrivateKey;
    fn signed(key: &RsaPrivateKey) -> Vec<u8> {
        let mut bytes = b"CNKL".to_vec();
        bytes.extend(36u32.to_le_bytes());
        bytes.extend([1, 1, 1, 0]);
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(36u64.to_le_bytes());
        bytes.extend(72u64.to_le_bytes());
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(Sha256::digest(b"abc"));
        let mut sig = key
            .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&bytes))
            .unwrap();
        sig.reverse();
        bytes.extend(sig);
        bytes
    }
    #[test]
    fn signature_and_tamper() {
        let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
        let bytes = signed(&key);
        assert_eq!(
            parse_with_key(&bytes, &key.to_public_key()).unwrap()[0].size,
            3
        );
        let mut changed = bytes.clone();
        changed[40] ^= 1;
        assert!(parse_with_key(&changed, &key.to_public_key()).is_err());
        assert!(parse_chunklist(&bytes).is_err());
    }
    #[test]
    fn method_two_rejected() {
        let mut bytes = vec![0u8; 328];
        bytes[..4].copy_from_slice(b"CNKL");
        bytes[4..8].copy_from_slice(&36u32.to_le_bytes());
        bytes[8] = 1;
        bytes[9] = 1;
        bytes[10] = 2;
        assert!(parse_chunklist(&bytes).is_err());
    }
    #[test]
    fn tokens_not_serialized() {
        let i = Info {
            name: "n".into(),
            board: "b".into(),
            product: "p".into(),
            image_url: "http://oscdn.apple.com/a".into(),
            chunklist_url: "http://oscdn.apple.com/c".into(),
            image_token: "SECRET".into(),
            chunklist_token: "SECRET".into(),
        };
        assert!(!serde_json::to_string(&i).unwrap().contains("SECRET"));
    }
}
