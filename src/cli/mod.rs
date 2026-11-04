use crate::{
    backend::{self, GdbServerOptions, Operation},
    gdbserver::GdbServer,
    manager::{Manager, Tool},
    session,
};
use std::path::PathBuf;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" {
        println!("nsc debugger management\n\n  list [openocd|pyocd|probe-rs]\n  install <tool> [version]\n  uninstall <tool> <version>\n  add-local <tool> <name> <directory>\n  remove-local <tool> <name>\n  use <tool> <version|system|local:name>\n  which <tool>\n  paths\n  gdbserver <tool> --target <target> [--interface <config>] [--port <port>] [--dry-run]\n  flash <tool> --target <target> --file <firmware> [--interface <config>] [--dry-run]\n  erase <tool> --target <target> [--interface <config>] [--bank <number>] [--dry-run]\n  reset <tool> --target <target> [--interface <config>] [--dry-run]\n  debug <tool> --target <target> --file <elf> [--interface <config>] [--port <port>] [--gdb <program>] [--dry-run]\n\nOpenOCD requires --interface; its --target is a target config. OpenOCD erase requires --bank. For pyOCD and probe-rs, --target is a chip name. Default port: 3333. Default GDB client: arm-none-eabi-gdb.");
        return Ok(());
    }
    let manager = Manager::new()?;
    match (args[0].as_str(), &args[1..]) {
        ("paths", []) => println!(
            "config: {}\ndata: {}",
            manager.config_path.display(),
            manager.data_dir.display()
        ),
        ("list", rest) if rest.len() <= 1 => {
            let tools = if rest.is_empty() {
                Tool::ALL.to_vec()
            } else {
                vec![rest[0].parse()?]
            };
            for tool in tools {
                let selected = manager.selected(tool)?;
                println!(
                    "{tool} (selected: {})",
                    selected.as_deref().unwrap_or("none")
                );
                for (version, path) in manager.installed(tool)? {
                    println!("  {version}: {}", path.display());
                }
                for (name, path) in manager.local(tool)? {
                    println!(
                        "  local:{name}: {}{}",
                        path.display(),
                        if path.is_file() { "" } else { " (missing)" }
                    );
                }
                for path in manager.system(tool) {
                    println!("  system: {} ({})", path.display(), manager.version(&path));
                }
            }
        }
        ("install", [tool, version]) => manager.install(tool.parse()?, version)?,
        ("install", [tool]) => manager.install_latest(tool.parse()?)?,
        ("add-local", [tool, name, directory]) => {
            manager.add_local(tool.parse()?, name, &PathBuf::from(directory))?
        }
        ("remove-local", [tool, name]) => manager.remove_local(tool.parse()?, name)?,
        ("uninstall", [tool, version]) | ("remove", [tool, version]) => {
            manager.uninstall(tool.parse()?, version)?
        }
        ("use", [tool, version]) => manager.select(tool.parse()?, version)?,
        ("which", [tool]) => println!("{}", manager.resolve(tool.parse()?)?.display()),
        ("gdbserver", [tool, rest @ ..]) => {
            let tool: Tool = tool.parse()?;
            let parsed = parse_options(rest)?;
            if parsed.file.is_some() || parsed.bank.is_some() || parsed.gdb.is_some() {
                return Err("gdbserver does not accept --file, --bank, or --gdb".into());
            }
            let options = parsed.server;
            let server = GdbServer::resolve(&manager, tool, options)?;
            let mut command = server.command()?;
            if parsed.dry_run {
                println!("{command:?}");
            } else {
                let status = command.status()?;
                if !status.success() {
                    return Err(format!("{tool} GDB server exited with {status}").into());
                }
            }
        }
        (action @ ("flash" | "erase" | "reset" | "debug"), [tool, rest @ ..]) => {
            let tool: Tool = tool.parse()?;
            let parsed = parse_options(rest)?;
            if action != "debug" && parsed.server.port != 3333 {
                return Err("--port is only valid for gdbserver or debug".into());
            }
            let executable = manager.resolve(tool)?;
            if action == "debug" {
                if parsed.bank.is_some() {
                    return Err("--bank is only valid for erase".into());
                }
                let elf = parsed.file.ok_or("debug requires --file <elf>")?;
                if !elf.is_file() {
                    return Err(format!("ELF file does not exist: {}", elf.display()).into());
                }
                let server = GdbServer::new(tool, &executable, parsed.server.clone())?;
                let command = server.command()?;
                let gdb = parsed.gdb.unwrap_or_else(|| "arm-none-eabi-gdb".into());
                if parsed.dry_run {
                    println!(
                        "server: {command:?}\nclient: {:?}",
                        session::gdb_command(&gdb, &elf, parsed.server.port)
                    );
                } else {
                    session::run(command, &gdb, &elf, parsed.server.port)?;
                }
            } else {
                if parsed.gdb.is_some() {
                    return Err("--gdb is only valid for debug".into());
                }
                let operation = match action {
                    "flash" => {
                        Operation::Flash(parsed.file.ok_or("flash requires --file <firmware>")?)
                    }
                    "erase" => {
                        if parsed.file.is_some() {
                            return Err("--file is only valid for flash or debug".into());
                        }
                        Operation::Erase { bank: parsed.bank }
                    }
                    "reset" => {
                        if parsed.file.is_some() || parsed.bank.is_some() {
                            return Err("reset does not accept --file or --bank".into());
                        }
                        Operation::Reset
                    }
                    _ => unreachable!(),
                };
                if action == "flash" && parsed.bank.is_some() {
                    return Err("--bank is only valid for erase".into());
                }
                let mut command =
                    backend::operation_command(tool, &executable, &parsed.server, &operation)?;
                if parsed.dry_run {
                    println!("{command:?}");
                } else {
                    let status = command.status()?;
                    if !status.success() {
                        return Err(format!("{tool} {action} exited with {status}").into());
                    }
                }
            }
        }
        _ => return Err("invalid arguments; run with --help".into()),
    }
    Ok(())
}

struct ParsedOptions {
    server: GdbServerOptions,
    dry_run: bool,
    file: Option<PathBuf>,
    bank: Option<u32>,
    gdb: Option<String>,
}

fn parse_options(args: &[String]) -> Result<ParsedOptions, Box<dyn std::error::Error>> {
    let mut target = None;
    let mut interface = None;
    let mut port = None;
    let mut dry_run = false;
    let mut file = None;
    let mut bank = None;
    let mut gdb = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        if flag == "--dry-run" {
            if dry_run {
                return Err("duplicate --dry-run".into());
            }
            dry_run = true;
            index += 1;
            continue;
        }
        if !matches!(
            flag,
            "--target" | "--interface" | "--port" | "--file" | "--bank" | "--gdb"
        ) {
            return Err(format!("unknown option: {flag}").into());
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if value.starts_with("--") {
            return Err(format!("missing value for {flag}").into());
        }
        match flag {
            "--target" if target.is_none() => target = Some(value.clone()),
            "--interface" if interface.is_none() => interface = Some(value.clone()),
            "--port" if port.is_none() => {
                port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|_| "invalid GDB port; use 1-65535")?,
                )
            }
            "--file" if file.is_none() => file = Some(PathBuf::from(value)),
            "--bank" if bank.is_none() => {
                bank = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| "invalid flash bank number")?,
                )
            }
            "--gdb" if gdb.is_none() => gdb = Some(value.clone()),
            _ => return Err(format!("duplicate {flag}").into()),
        }
        index += 2;
    }
    Ok(ParsedOptions {
        server: GdbServerOptions {
            target: target.ok_or("missing --target")?,
            interface,
            port: port.unwrap_or(3333),
        },
        dry_run,
        file,
        bank,
        gdb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_server_options() {
        let args = ["--port", "4444", "--target", "STM32F407VG", "--dry-run"].map(str::to_owned);
        let parsed = parse_options(&args).unwrap();
        assert_eq!(parsed.server.port, 4444);
        assert_eq!(parsed.server.target, "STM32F407VG");
        assert!(parsed.dry_run);
        assert!(
            parse_options(&["--target".into(), "a".into(), "--target".into(), "b".into()]).is_err()
        );
        assert!(parse_options(&["--port".into(), "0".into()]).is_err());
    }
}
