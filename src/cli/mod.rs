use crate::manager::{Manager, Tool};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" {
        println!("nsc debugger management\n\n  list [openocd|pyocd|probe-rs]\n  install <openocd|pyocd|probe-rs> [version]\n  uninstall <tool> <version>\n  use <tool> <version|system>\n  which <tool>\n  paths\n\nOmit version to install the latest stable release. OpenOCD versions are xPack release versions, e.g. 0.12.0-7.");
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
                for path in manager.system(tool) {
                    println!("  system: {} ({})", path.display(), manager.version(&path));
                }
            }
        }
        ("install", [tool, version]) => manager.install(tool.parse()?, version)?,
        ("install", [tool]) => manager.install_latest(tool.parse()?)?,
        ("uninstall", [tool, version]) | ("remove", [tool, version]) => {
            manager.uninstall(tool.parse()?, version)?
        }
        ("use", [tool, version]) => manager.select(tool.parse()?, version)?,
        ("which", [tool]) => println!("{}", manager.resolve(tool.parse()?)?.display()),
        _ => return Err("invalid arguments; run with --help".into()),
    }
    Ok(())
}
