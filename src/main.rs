use cli::{BuildOptions, BuildProfile};
use craw::*;
use std::env;
use std::fs;

/// Links `file` with every `.craw` module it imports and transpiles the
/// program. Returns the Rust source and every file it was built from.
fn transpile_file(file: &str) -> Result<(String, Vec<std::path::PathBuf>), String> {
    if !std::path::Path::new(file).exists() {
        return Err(format!("Failed to read file '{}'", file));
    }
    let loader = modules::FsLoader::default();
    match modules::parse_program(std::path::Path::new(file), &[], &loader) {
        Ok((ast, linked)) => Ok((transpiler::transpile(&ast), linked.files)),
        Err(errors) => {
            eprintln!("{}", modules::format_errors(&errors));
            Err("Compilation failed".to_string())
        }
    }
}

fn build_file(options: BuildOptions) -> Result<Option<i32>, String> {
    let file_path = std::path::Path::new(&options.file);
    let stem = file_path
        .file_stem()
        .ok_or_else(|| format!("Invalid input file path: {}", options.file))?
        .to_string_lossy()
        .to_string();
    let out_dir = file_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(".craw");
    fs::create_dir_all(&out_dir).map_err(|e| format!("Failed to create build directory: {}", e))?;
    let out_rs = out_dir.join(format!("{}.rs", stem));
    let out_bin = out_dir.join(format!("{}{}", stem, std::env::consts::EXE_SUFFIX));
    let profile_marker = out_dir.join(format!(
        "{}.{}",
        stem,
        if options.profile == BuildProfile::Release {
            "release"
        } else {
            "debug"
        }
    ));

    // Linking is cheap and tells us every file the program is built from,
    // so an edit to any imported module invalidates the cached binary.
    let (transpiled, sources) = transpile_file(&options.file)?;
    let newest_source = sources
        .iter()
        .filter_map(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        .max();

    let mut needs_build = true;
    if out_bin.exists()
        && profile_marker.exists()
        && let Ok(bin_meta) = fs::metadata(&out_bin)
        && let (Ok(bin_mtime), Some(src_mtime)) = (bin_meta.modified(), newest_source)
        && bin_mtime >= src_mtime
    {
        needs_build = false;

        // Also check compiler mtime: if craw was updated, recompile.
        if let Ok(exe_path) = std::env::current_exe()
            && let Ok(exe_meta) = fs::metadata(exe_path)
            && let Ok(exe_mtime) = exe_meta.modified()
            && exe_mtime > bin_mtime
        {
            needs_build = true;
        }
    }

    if needs_build {
        let full_rs = format!(
            r#"#![allow(warnings)]
{}

{}

fn main() {{
    craw_main();
}}
"#,
            runtime_template::RUNTIME_TEMPLATE,
            transpiled
        );

        fs::write(&out_rs, full_rs).map_err(|e| format!("Failed to write output file: {}", e))?;

        let mut cmd = std::process::Command::new("rustc");
        cmd.arg(&out_rs)
            .arg("-o")
            .arg(&out_bin)
            .arg("--edition=2024");

        if options.profile == BuildProfile::Release {
            cmd.args([
                "-C",
                "opt-level=3",
                "-C",
                "lto",
                "-C",
                "codegen-units=1",
                "-C",
                "strip=symbols",
            ]);
        }

        let status = cmd
            .status()
            .map_err(|e| format!("Failed to execute rustc: {}", e))?;

        if !status.success() {
            return Err("rustc compilation failed".to_string());
        }

        // Mark which profile this binary was built for, and remove any
        // stale marker from the other profile so switching --release on or
        // off always triggers a fresh build instead of reusing the wrong
        // binary.
        let other_marker = out_dir.join(format!(
            "{}.{}",
            stem,
            if options.profile == BuildProfile::Release {
                "debug"
            } else {
                "release"
            }
        ));
        let _ = fs::remove_file(&other_marker);
        fs::write(&profile_marker, "")
            .map_err(|e| format!("Failed to write build marker: {}", e))?;
    }

    if options.run {
        let current_dir = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;
        let bin_path = current_dir.join(&out_bin);
        let run_status = std::process::Command::new(bin_path)
            .status()
            .map_err(|e| format!("Failed to execute binary: {}", e))?;

        return Ok(Some(exit_code(run_status)));
    }

    Ok(None)
}

/// The process exit code to propagate for a finished child. A child
/// killed by a signal has no exit code; report it the way shells do
/// (`128 + signal`) rather than as success.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("craw {}", env!("CARGO_PKG_VERSION"));
        println!("The Craw language compiler and toolchain.");
        println!("");
        println!("USAGE:");
        println!("    craw <COMMAND> [ARGS]");
        println!("");
        println!("COMMANDS:");
        println!("    new <name>              Create a new project");
        println!("    transpile <file> [out]  Transpile a file to Rust");
        println!("    build <file>            Build a file");
        println!("    run <file> [--release]  Build and run a file");
        println!("");
        println!("FLAGS:");
        println!("    -h, --help      Print help information");
        println!("    -v, --version   Print version information");
        std::process::exit(1);
    }

    // Convert to Vec<&str> for the parser
    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let command = match cli::parse_args(args_str) {
        Ok(cmd) => cmd,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    match command {
        cli::Command::Transpile { file, out } => {
            let transpiled = match transpile_file(&file) {
                Ok((t, _)) => t,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            };
            let out_file = out.unwrap_or_else(|| {
                let file_path = std::path::Path::new(&file);
                file_path.with_extension("rs").to_str().unwrap().to_string()
            });
            if let Err(e) = fs::write(&out_file, transpiled) {
                eprintln!("Error: Failed to write output file: {}", e);
                std::process::exit(1);
            }
            println!("Transpiled {} to {}", file, out_file);
        }
        cli::Command::New { name } => {
            let result: Result<(), String> = (|| {
                if std::path::Path::new(&name).exists() {
                    return Err(format!("'{}' already exists", name));
                }
                std::fs::create_dir(&name)
                    .map_err(|e| format!("Failed to create project directory: {}", e))?;
                std::fs::create_dir(format!("{}/src", name))
                    .map_err(|e| format!("Failed to create src directory: {}", e))?;

                let main_craw = include_str!("templates/new_project_main.craw.template");
                let main_rs = include_str!("templates/new_project_main.rs.template");
                let build_rs = include_str!("templates/new_project_build.rs.template");
                let cargo_toml_tmpl = include_str!("templates/new_project_cargo.toml.template");
                let cargo_toml = cargo_toml_tmpl.replace("{}", &name);

                std::fs::write(format!("{}/Cargo.toml", name), cargo_toml)
                    .map_err(|e| format!("Failed to write Cargo.toml: {}", e))?;

                std::fs::write(format!("{}/build.rs", name), build_rs)
                    .map_err(|e| format!("Failed to write build.rs: {}", e))?;

                std::fs::write(format!("{}/src/main.craw", name), main_craw)
                    .map_err(|e| format!("Failed to write main.craw: {}", e))?;

                std::fs::write(format!("{}/src/main.rs", name), main_rs)
                    .map_err(|e| format!("Failed to write main.rs: {}", e))?;

                let runtime_file = runtime_template::RUNTIME_TEMPLATE;
                std::fs::write(format!("{}/src/runtime.rs", name), runtime_file)
                    .map_err(|e| format!("Failed to write runtime.rs: {}", e))?;
                Ok(())
            })();

            if let Err(e) = result {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }

            println!("Created new Craw project: {}", name);
        }
        cli::Command::Build { file } => {
            let options = cli::BuildOptions {
                file,
                profile: cli::BuildProfile::Release,
                run: false,
            };
            if let Err(e) = build_file(options) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        cli::Command::Run { file, release } => {
            let options = cli::BuildOptions {
                file,
                profile: if release {
                    cli::BuildProfile::Release
                } else {
                    cli::BuildProfile::Debug
                },
                run: true,
            };
            match build_file(options) {
                Ok(Some(code)) => std::process::exit(code),
                Ok(None) => {}
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        cli::Command::Help => {
            println!("craw {}", env!("CARGO_PKG_VERSION"));
            println!("The Craw language compiler and toolchain.");
            println!("");
            println!("USAGE:");
            println!("    craw <COMMAND> [ARGS]");
            println!("");
            println!("COMMANDS:");
            println!("    new <name>              Create a new project");
            println!("    transpile <file> [out]  Transpile a file to Rust");
            println!("    build <file>            Build a file");
            println!("    run <file> [--release]  Build and run a file");
            println!("");
            println!("FLAGS:");
            println!("    -h, --help      Print help information");
            println!("    -v, --version   Print version information");
        }
        cli::Command::Version => {
            println!("craw {}", env!("CARGO_PKG_VERSION"));
        }
    }
}
