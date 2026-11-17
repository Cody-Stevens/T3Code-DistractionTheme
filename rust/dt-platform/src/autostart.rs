//! Per-user start-at-login entry (HKCU Run key). Uses the system `reg.exe` without a shell, so no
//! registry FFI is needed; values are passed as separate arguments.

use dt_core::{Error, Result};
use std::process::{Command, Stdio};

/// Registry key whose values Windows runs at sign-in for the current user.
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// Arguments for `reg.exe` that set `name` to `command_line` under [`RUN_KEY`].
pub fn enable_args(name: &str, command_line: &str) -> Vec<String> {
    [
        "add",
        RUN_KEY,
        "/v",
        name,
        "/t",
        "REG_SZ",
        "/d",
        command_line,
        "/f",
    ]
    .map(String::from)
    .to_vec()
}

/// Arguments for `reg.exe` that delete `name` from [`RUN_KEY`].
pub fn disable_args(name: &str) -> Vec<String> {
    ["delete", RUN_KEY, "/v", name, "/f"]
        .map(String::from)
        .to_vec()
}

/// Arguments for `reg.exe` that read `name` from [`RUN_KEY`].
pub fn query_args(name: &str) -> Vec<String> {
    ["query", RUN_KEY, "/v", name].map(String::from).to_vec()
}

/// Start `command_line` at sign-in. Replaces an existing value of the same name.
pub fn enable(name: &str, command_line: &str) -> Result<()> {
    if reg(&enable_args(name, command_line))?.0 {
        Ok(())
    } else {
        Err(Error::Protocol("could not write the start-at-login entry"))
    }
}

/// Remove the sign-in entry. Returns false when there was none.
pub fn disable(name: &str) -> Result<bool> {
    if query(name)?.is_none() {
        return Ok(false);
    }
    if reg(&disable_args(name))?.0 {
        Ok(true)
    } else {
        Err(Error::Protocol("could not remove the start-at-login entry"))
    }
}

/// The command line registered under `name`, if any.
pub fn query(name: &str) -> Result<Option<String>> {
    let (ok, out) = reg(&query_args(name))?;
    Ok(if ok { parse_query(name, &out) } else { None })
}

/// Pull the REG_SZ data for `name` out of `reg query` output.
pub fn parse_query(name: &str, output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix(name)?;
        let rest = rest.trim_start().strip_prefix("REG_SZ")?;
        Some(rest.trim().to_string())
    })
}

fn reg(args: &[String]) -> Result<(bool, String)> {
    let mut command = Command::new("reg.exe");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = command.output()?;
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_keep_the_command_line_as_one_value() {
        let line = r#""C:\Users\me\AppData\Local\GlowTube\bin\distraction-service.exe""#;
        let args = enable_args("GlowTube", line);
        assert_eq!(args[0], "add");
        assert_eq!(args[1], RUN_KEY);
        assert_eq!(args[3], "GlowTube");
        assert_eq!(args[7], line);
        assert_eq!(args.last().unwrap(), "/f");
        assert_eq!(
            disable_args("GlowTube"),
            ["delete", RUN_KEY, "/v", "GlowTube", "/f"]
        );
    }

    #[test]
    fn query_output_is_parsed() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n    \
                   GlowTube    REG_SZ    \"C:\\x y\\distraction-service.exe\"\r\n\r\n";
        assert_eq!(
            parse_query("GlowTube", out).as_deref(),
            Some(r#""C:\x y\distraction-service.exe""#)
        );
        assert_eq!(parse_query("Other", out), None);
    }
}
