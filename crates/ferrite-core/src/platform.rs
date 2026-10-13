//! Host-specific utility names and supported write operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Windows,
    Linux,
    MacOs,
    Other,
}
pub const fn host() -> Host {
    if cfg!(target_os = "windows") {
        Host::Windows
    } else if cfg!(target_os = "linux") {
        Host::Linux
    } else if cfg!(target_os = "macos") {
        Host::MacOs
    } else {
        Host::Other
    }
}
pub const fn host_name() -> &'static str {
    match host() {
        Host::Windows => "Windows",
        Host::Linux => "Linux",
        Host::MacOs => "macOS",
        Host::Other => "Unknown",
    }
}
pub const fn physical_usb_supported() -> bool {
    matches!(host(), Host::Windows | Host::Linux | Host::MacOs)
}
pub fn utility_name(tool: &str, platform: Host) -> anyhow::Result<&'static str> {
    Ok(match (tool, platform) {
        ("macserial", Host::Windows) => "macserial.exe",
        ("macserial", Host::Linux) => "macserial.linux",
        ("macserial", Host::MacOs) => "macserial",
        ("ocvalidate", Host::Windows) => "ocvalidate.exe",
        ("ocvalidate", Host::Linux) => "ocvalidate.linux",
        ("ocvalidate", Host::MacOs) => "ocvalidate",
        _ => anyhow::bail!("Unsupported utility/host"),
    })
}
pub fn default_iasl() -> String {
    if cfg!(target_os = "macos") {
        for path in ["/opt/homebrew/bin/iasl", "/usr/local/bin/iasl"] {
            if std::path::Path::new(path).is_file() {
                return path.into();
            }
        }
    }
    "iasl".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_uses_native_tools() {
        assert_eq!(
            utility_name("ocvalidate", Host::MacOs).unwrap(),
            "ocvalidate"
        );
        assert_eq!(utility_name("macserial", Host::MacOs).unwrap(), "macserial");
        assert!(utility_name("unknown", Host::MacOs).is_err());
    }
}
