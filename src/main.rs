use chumsky::Parser;
use cli::{BuildOptions, BuildProfile};
use craw::*;
use std::env;
use std::fs;

fn transpile_file(file: &str) -> Result<String, String> {
    let input =
        fs::read_to_string(file).map_err(|e| format!("Failed to read file '{}': {}", file, e))?;
    let mut lexer = lexer::Lexer::new(&input);
    let tokens = lexer.tokenize();
    // println!("Tokens: {:?}", tokens);
    let (ast, errors) = parser::parser().parse(&tokens).into_output_errors();
    if !errors.is_empty() {
        for err in &errors {
            eprintln!("Parse error: {:?}", err);
            let span = err.span();
            eprintln!(
                "Tokens around error: {:?}",
                &tokens[span.start.saturating_sub(5)..std::cmp::min(span.end + 5, tokens.len())]
            );
        }
        return Err("Parsing failed".to_string());
    }
    let ast = ast.unwrap_or_default();
    Ok(transpiler::transpile(&ast))
}

fn build_file(options: BuildOptions) -> Result<(), String> {
    let file_path = std::path::Path::new(&options.file);
    let out_rs = file_path.with_extension("rs");
    let out_bin = if std::env::consts::EXE_EXTENSION.is_empty() {
        file_path.with_extension("")
    } else {
        file_path.with_extension(std::env::consts::EXE_EXTENSION)
    };

    let mut needs_build = true;
    if out_bin.exists()
        && out_bin != file_path
        && let Ok(bin_meta) = fs::metadata(&out_bin)
        && let Ok(src_meta) = fs::metadata(&options.file)
        && let (Ok(bin_mtime), Ok(src_mtime)) = (bin_meta.modified(), src_meta.modified())
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
        let transpiled = transpile_file(&options.file)?;
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
        cmd.arg(&out_rs).arg("-o").arg(&out_bin);

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
    }

    if options.run {
        let current_dir = std::env::current_dir()
            .map_err(|e| format!("Failed to get current directory: {}", e))?;
        let bin_path = current_dir.join(&out_bin);
        let run_status = std::process::Command::new(bin_path)
            .status()
            .map_err(|e| format!("Failed to execute binary: {}", e))?;

        if !run_status.success() {
            return Err("Program execution failed".to_string());
        }
    }

    Ok(())
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
                Ok(t) => t,
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
            std::fs::create_dir(&name).expect("Failed to create project directory");
            std::fs::create_dir(format!("{}/src", name)).expect("Failed to create src directory");

            let main_craw = include_str!("templates/new_project_main.craw.template");
            let main_rs = include_str!("templates/new_project_main.rs.template");
            let build_rs = include_str!("templates/new_project_build.rs.template");
            let cargo_toml_tmpl = include_str!("templates/new_project_cargo.toml.template");
            let cargo_toml = cargo_toml_tmpl.replace("{}", &name);

            std::fs::write(format!("{}/Cargo.toml", name), cargo_toml)
                .expect("Failed to write Cargo.toml");

            std::fs::write(format!("{}/build.rs", name), build_rs)
                .expect("Failed to write build.rs");

            std::fs::write(format!("{}/src/main.craw", name), main_craw)
                .expect("Failed to write main.craw");

            std::fs::write(format!("{}/src/main.rs", name), main_rs)
                .expect("Failed to write main.rs");

            let runtime_file = runtime_template::RUNTIME_TEMPLATE;
            std::fs::write(format!("{}/src/runtime.rs", name), runtime_file)
                .expect("Failed to write runtime.rs");

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
            if let Err(e) = build_file(options) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
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
