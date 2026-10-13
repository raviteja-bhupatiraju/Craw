import os
import subprocess
import sys

def run_cmd(cmd):
    sys.stdout.flush()
    sys.stderr.flush()
    subprocess.run(cmd, check=False)

def main():
    print("==============================================")
    print("      Craw Performance Benchmarks Report      ")
    print("==============================================\n")

    # 1. Run Python
    print("Running Python Benchmark...")
    run_cmd(["pixi", "run", "python", "benchmarks/bench_python.py"])
    print("\n----------------------------------------------\n")

    # 2. Run Rust
    print("Compiling and Running Rust Benchmark...")
    rust_exe = "benchmarks/bench_rust"
    if os.name == 'nt':
        rust_exe += ".exe"
    
    # Compile with optimizations
    subprocess.run(["rustc", "-C", "opt-level=3", "benchmarks/bench_rust.rs", "-o", rust_exe], check=True)
    run_cmd([f"./{rust_exe}" if os.name != 'nt' else rust_exe])
    print("\n----------------------------------------------\n")

    # 3. Run Craw Untyped
    print("Running Craw Benchmark (Untyped) (with --release)...")
    run_cmd(["cargo", "run", "--release", "--bin", "craw", "--", "run", "--release", "benchmarks/bench_craw_untyped.craw"])
    print("\n----------------------------------------------\n")

    # 4. Run Craw Native
    print("Running Craw Benchmark (Native) (with --release)...")
    run_cmd(["cargo", "run", "--release", "--bin", "craw", "--", "run", "--release", "benchmarks/bench_craw.craw"])
    print("\n==============================================")

if __name__ == "__main__":
    main()
