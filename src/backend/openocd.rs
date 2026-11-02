use super::{GdbServerOptions, Operation, Result};
use std::{path::Path, process::Command};

pub fn gdbserver_command(executable: &Path, options: &GdbServerOptions) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("-f")
        .arg(
            options
                .interface
                .as_deref()
                .expect("validated OpenOCD interface"),
        )
        .arg("-f")
        .arg(&options.target)
        .arg("-c")
        .arg(format!("gdb_port {}", options.port));
    command
}

pub fn operation_command(
    executable: &Path,
    options: &GdbServerOptions,
    operation: &Operation,
) -> Result<Command> {
    let mut command = Command::new(executable);
    command
        .arg("-f")
        .arg(options.interface.as_deref().expect("validated interface"))
        .arg("-f")
        .arg(&options.target);
    let script = match operation {
        Operation::Flash(path) => {
            // Tcl braces preserve spaces and Windows backslashes. Reject an unmatched
            // brace or line break rather than letting a path become Tcl code.
            let path = path.to_string_lossy();
            if path.contains(['{', '}', '\n', '\r']) {
                return Err("OpenOCD firmware path cannot contain braces or line breaks".into());
            }
            format!("program {{{path}}} verify reset exit")
        }
        Operation::Erase { bank: Some(bank) } => {
            format!("init; reset halt; flash erase_sector {bank} 0 last; shutdown")
        }
        Operation::Erase { bank: None } => unreachable!("validated bank"),
        Operation::Reset => "init; reset run; shutdown".to_owned(),
    };
    command.arg("-c").arg(script);
    Ok(command)
}

/// Adapter for OpenOCD middleware.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenOcd;

impl super::DebugBackend for OpenOcd {
    fn tool(&self) -> crate::manager::Tool {
        crate::manager::Tool::OpenOcd
    }

    fn gdbserver_command(
        &self,
        executable: &Path,
        options: &GdbServerOptions,
    ) -> super::Result<Command> {
        options.validate(self.tool())?;
        Ok(gdbserver_command(executable, options))
    }

    fn operation_command(
        &self,
        executable: &Path,
        options: &GdbServerOptions,
        operation: &Operation,
    ) -> super::Result<Command> {
        super::validate_operation(self.tool(), options, operation)?;
        operation_command(executable, options, operation)
    }
}
