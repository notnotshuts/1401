use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub const MAX_REPORT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub hardware: Value,
    pub sha256: String,
}
impl Report {
    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)
            .with_context(|| format!("Не удалось открыть {}", path.display()))?
            .take(MAX_REPORT + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_REPORT {
            bail!("Отчёт превышает 16 MiB");
        }
        let json = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let mut hardware: Value =
            serde_json::from_slice(json).context("Некорректный JSON отчёта")?;
        let obj = hardware
            .as_object_mut()
            .context("Отчёт должен быть JSON-объектом")?;
        for section in [
            "CPU",
            "Motherboard",
            "BIOS",
            "GPU",
            "Network",
            "Sound",
            "Storage Controllers",
            "Bluetooth",
            "SD Controller",
            "Biometric",
            "Monitor",
        ] {
            let value = obj
                .entry(section)
                .or_insert_with(|| Value::Object(Map::new()));
            if !value.is_object() {
                bail!("Раздел {section} должен быть объектом");
            }
        }
        // Never trust compatibility verdicts embedded in imported reports.
        for (section, entries) in obj.iter_mut() {
            if matches!(section.as_str(), "CPU" | "Motherboard" | "BIOS") {
                if let Some(p) = entries.as_object_mut() {
                    p.remove("Compatibility");
                    p.remove("OCLP Compatibility");
                }
            } else if let Some(rows) = entries.as_object_mut() {
                for properties in rows.values_mut() {
                    if let Some(p) = properties.as_object_mut() {
                        p.remove("Compatibility");
                        p.remove("OCLP Compatibility");
                    }
                }
            }
        }
        Ok(Self {
            hardware,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }
    pub fn rows(&self, section: &str) -> Vec<(&str, &Value)> {
        self.hardware[section]
            .as_object()
            .map(|m| m.iter().map(|(n, p)| (n.as_str(), p)).collect())
            .unwrap_or_default()
    }
    pub fn text<'a>(&'a self, section: &str, key: &str) -> &'a str {
        text(&self.hardware[section], key)
    }
}
pub fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_schema() {
        assert!(Report::from_bytes(b"[]").is_err());
        assert!(Report::from_bytes(br#"{"GPU": []}"#).is_err());
    }
    #[test]
    fn strips_untrusted_verdict_and_accepts_bom() {
        let r = Report::from_bytes(b"\xef\xbb\xbf{\"CPU\":{\"Compatibility\":[\"25\",\"17\"]}}")
            .unwrap();
        assert!(r.hardware["CPU"].get("Compatibility").is_none());
        assert_eq!(r.sha256.len(), 64);
    }
}
