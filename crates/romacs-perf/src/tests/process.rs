//! Single-child deadline support for the independent GNU fixture regressions.
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;
use wait_timeout::ChildExt;

#[derive(Debug)]
pub enum CommandError {
    Launch(std::io::Error),
    TimedOut(Output),
    Capture(String),
}

pub fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, CommandError> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(CommandError::Launch)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CommandError::Capture("stdout pipe was not created".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CommandError::Capture("stderr pipe was not created".to_string()))?;
    let stdout_reader = thread::spawn(move || read_pipe(stdout));
    let stderr_reader = thread::spawn(move || read_pipe(stderr));
    let (status, timed_out) = match child.wait_timeout(timeout).map_err(CommandError::Launch)? {
        Some(status) => (status, false),
        None => {
            let _ = child.kill();
            (child.wait().map_err(CommandError::Launch)?, true)
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| CommandError::Capture("stdout reader panicked".to_string()))?
        .map_err(|error| CommandError::Capture(format!("failed to read stdout: {error}")))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| CommandError::Capture("stderr reader panicked".to_string()))?
        .map_err(|error| CommandError::Capture(format!("failed to read stderr: {error}")))?;
    let output = Output {
        status,
        stdout,
        stderr,
    };
    if timed_out {
        Err(CommandError::TimedOut(output))
    } else {
        Ok(output)
    }
}

fn read_pipe(mut pipe: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}
