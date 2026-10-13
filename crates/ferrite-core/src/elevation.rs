//! Elevate only the CLI writer, never the GUI. Arguments are data, not shell code.
#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use anyhow::bail;
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::time::Duration;
pub fn write_helper(_cli: &Path, _args: &[String]) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    let (cli, args) = (_cli, _args);
    #[cfg(target_os = "linux")]
    {
        let mut argv = vec![cli.to_str().context("CLI path is not UTF-8")?];
        argv.extend(args.iter().map(String::as_str));
        crate::workspace::run_bounded("pkexec",&argv,Duration::from_secs(1800)).context("Привилегированный helper не завершился. Если PolicyKit недоступен, запустите ferrite-cli write через sudo вручную.")?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
            UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL},
        };
        fn quote(arg: &str) -> String {
            let mut s = String::from("\"");
            let mut slashes = 0;
            for c in arg.chars() {
                if c == '\\' {
                    slashes += 1;
                } else if c == '"' {
                    s.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                    s.push('"');
                    slashes = 0;
                } else {
                    s.extend(std::iter::repeat_n('\\', slashes));
                    s.push(c);
                    slashes = 0;
                }
            }
            s.extend(std::iter::repeat_n('\\', slashes * 2));
            s.push('"');
            s
        }
        let file: Vec<u16> = cli.as_os_str().encode_wide().chain([0]).collect();
        let params = args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
        let params: Vec<u16> = params.encode_utf16().chain([0]).collect();
        let verb: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        // SAFETY: All UTF-16 strings live for the synchronous ShellExecuteEx call.
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if info.hProcess.is_null() {
            bail!("UAC did not return a helper process");
        }
        let waited = unsafe { WaitForSingleObject(info.hProcess, 1_800_000) };
        let mut code = 1;
        let got = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
        unsafe {
            CloseHandle(info.hProcess);
        }
        if waited != 0 || got == 0 || code != 0 {
            bail!("Windows helper stopped or was cancelled (exit {code})");
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let cli = std::fs::canonicalize(cli).context("Missing CLI helper")?;
        if !cli.is_file() {
            anyhow::bail!("Missing CLI helper");
        }
        let mut argv = vec![
            "-e",
            MACOS_ADMIN_SCRIPT,
            "--",
            cli.to_str().context("CLI path must be UTF-8")?,
        ];
        argv.extend(args.iter().map(String::as_str));
        crate::workspace::run_bounded("/usr/bin/osascript", &argv, Duration::from_secs(1800))
            .context("macOS authorization cancelled or USB writer failed")?;
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        bail!("Elevation is supported only on Windows/Linux/macOS")
    }
}
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;

#[cfg(target_os = "macos")]
const MACOS_ADMIN_SCRIPT: &str = r#"on run argv
    set commandText to ""
    repeat with argument in argv
        set commandText to commandText & quoted form of (argument as text) & " "
    end repeat
    with timeout of 1800 seconds
        do shell script commandText with administrator privileges
    end timeout
end run"#;

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    #[test]
    fn authorization_command_quotes_arguments_without_executing_them() {
        let script = MACOS_ADMIN_SCRIPT.replace(
            "do shell script commandText with administrator privileges",
            "return commandText",
        );
        let arguments = [
            "/Applications/Ferrite's app/ferrite-cli",
            "a; echo injected",
            "$(touch /tmp/ferrite-unwanted)",
            "a\\\"b",
            "line\nbreak",
        ];
        let mut argv = vec!["-e", script.as_str(), "--"];
        argv.extend(arguments);
        let result =
            crate::workspace::run_bounded("/usr/bin/osascript", &argv, Duration::from_secs(15))
                .unwrap();
        let expected = arguments
            .iter()
            .map(|a| format!("'{}' ", a.replace('\'', "'\\''")))
            .collect::<String>();
        assert_eq!(
            String::from_utf8(result).unwrap().trim_end_matches('\n'),
            expected
        );
    }
}
