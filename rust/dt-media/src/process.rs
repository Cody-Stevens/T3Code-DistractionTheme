//! Hidden, owned tool processes; diagnostics never include tool output.

use dt_core::{Error, Result};
use std::process::{Child, Command, ExitStatus, Stdio};

pub(crate) fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

pub(crate) fn check(status: ExitStatus, program: &'static str) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(Error::ExternalTool {
            program,
            code: status.code(),
        })
    }
}

pub(crate) struct OwnedChild(pub(crate) Child);

impl OwnedChild {
    pub(crate) fn terminate(&mut self) -> std::io::Result<()> {
        if self.0.try_wait()?.is_none() {
            #[cfg(windows)]
            {
                let status = command("taskkill")
                    .args(["/T", "/F", "/PID", &self.0.id().to_string()])
                    .stdout(Stdio::null())
                    .status()?;
                if !status.success() && self.0.try_wait()?.is_none() {
                    return Err(std::io::Error::other(
                        "could not terminate media process tree",
                    ));
                }
            }
            #[cfg(not(windows))]
            self.0.kill()?;
        }
        self.0.wait()?;
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}
