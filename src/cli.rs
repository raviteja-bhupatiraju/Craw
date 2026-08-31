#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BuildProfile {
    Debug,
    Release,
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub file: String,
    pub profile: BuildProfile,
    pub run: bool,
}

#[derive(Debug)]
pub enum Command {
    New { name: String },
    Transpile { file: String, out: Option<String> },
    Build { file: String },
    Run { file: String, release: bool },
    Help,
    Version,
}

pub fn parse_args(args: Vec<&str>) -> Result<Command, String> {
    if args.len() < 2 {
        return Err("No command provided".to_string());
    }

    if args.contains(&"--help") || args.contains(&"-h") {
        return Ok(Command::Help);
    }
    if args.contains(&"--version") || args.contains(&"-v") {
        return Ok(Command::Version);
    }

    match args[1] {
        "new" => {
            let name = args.get(2).ok_or("Missing project name")?.to_string();
            Ok(Command::New { name })
        }
        "transpile" => {
            let file = args.get(2).ok_or("Missing input file")?.to_string();
            let out = args.get(3).map(|s| s.to_string());
            Ok(Command::Transpile { file, out })
        }
        "build" => {
            let file = args.get(2).ok_or("Missing input file")?.to_string();
            Ok(Command::Build { file })
        }
        "run" => {
            let release = args.contains(&"--release") || args.contains(&"-r");
            let file = args
                .into_iter()
                .skip(2)
                .find(|arg| *arg != "--release" && *arg != "-r")
                .ok_or("Missing input file")?
                .to_string();

            Ok(Command::Run { file, release })
        }
        cmd => Err(format!("Unknown command: {}", cmd)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        let args = parse_args(vec!["craw", "new", "my_app"]).unwrap();
        assert!(matches!(args, Command::New { name } if name == "my_app"));
    }

    #[test]
    fn test_cli_parsing_build_run() {
        let args_build = parse_args(vec!["craw", "build", "script.craw"]).unwrap();
        assert!(matches!(args_build, Command::Build { file } if file == "script.craw"));

        let args_run = parse_args(vec!["craw", "run", "script.craw"]).unwrap();
        assert!(
            matches!(args_run, Command::Run { file, release } if file == "script.craw" && !release)
        );

        let args_run_rel = parse_args(vec!["craw", "run", "--release", "script.craw"]).unwrap();
        assert!(
            matches!(args_run_rel, Command::Run { file, release } if file == "script.craw" && release)
        );

        let args_run_r = parse_args(vec!["craw", "run", "-r", "script.craw"]).unwrap();
        assert!(
            matches!(args_run_r, Command::Run { file, release } if file == "script.craw" && release)
        );

        let args_run_rel_after =
            parse_args(vec!["craw", "run", "script.craw", "--release"]).unwrap();
        assert!(
            matches!(args_run_rel_after, Command::Run { file, release } if file == "script.craw" && release)
        );
    }

    #[test]
    fn test_cli_parsing_help_version() {
        let args_help = parse_args(vec!["craw", "--help"]).unwrap();
        assert!(matches!(args_help, Command::Help));

        let args_h = parse_args(vec!["craw", "-h"]).unwrap();
        assert!(matches!(args_h, Command::Help));

        let args_version = parse_args(vec!["craw", "--version"]).unwrap();
        assert!(matches!(args_version, Command::Version));

        let args_v = parse_args(vec!["craw", "-v"]).unwrap();
        assert!(matches!(args_v, Command::Version));
    }

    #[test]
    fn test_cli_parsing_missing_args() {
        let err = parse_args(vec!["craw", "build"]).unwrap_err();
        assert_eq!(err, "Missing input file");
    }
}
