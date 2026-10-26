#shellcheck shell=bash
# End-to-end specs for Rust graph targets.

Describe 'rust graph'
  BeforeEach 'setup_workspace'
  AfterEach 'cleanup_workspace'

  rust_toolchain_unavailable() {
    command -v cargo >/dev/null 2>&1 || return 0
    command -v rustc >/dev/null 2>&1 || return 0
    return 1
  }

  create_failing_build_script_fixture() {
    mkdir -p "$WORKSPACE/crates/failing/src"
    cat > "$WORKSPACE/once.toml" <<'TOML'
[workspace]
include = ["crates/*/once.toml"]
TOML
    cat > "$WORKSPACE/crates/failing/once.toml" <<'TOML'
[[target]]
name = "failing"
kind = "rust_library"
srcs = ["src/**/*.rs"]

[target.attrs]
crate_name = "failing"
crate_root = "src/lib.rs"
build_script = "build.rs"
edition = "2021"
TOML
    printf 'pub fn value() -> i32 { 1 }\n' > "$WORKSPACE/crates/failing/src/lib.rs"
    cat > "$WORKSPACE/crates/failing/build.rs" <<'RS'
fn main() {
    println!("build script stdout is visible");
    eprintln!("build script stderr is visible");
    std::process::exit(7);
}
RS
  }

  It 'surfaces captured build script output when a declared action fails'
    create_failing_build_script_fixture

    When call once build crates/failing/failing
    The status should be failure
    The stderr should include 'crates/failing/failing:build-script'
    The stderr should include 'stdout:'
    The stderr should include 'build script stdout is visible'
    The stderr should include 'build script stderr is visible'
    The stderr should not include 'last 16384 bytes of stderr'
  End

  It 'records evidence when a declared build action fails'
    create_failing_build_script_fixture
    once build crates/failing/failing >/dev/null 2>&1 || true

    When call once --format json query evidence crates/failing/failing:build
    The status should be success
    The stdout should include '"subject":{"kind":"target","id":"crates/failing/failing","capability":"build"}'
    The stdout should include '"status":"failed"'
    The stdout should include '"cache":"miss"'
    The stdout should include '"exit_code":7'
    The stdout should include '"stdout"'
    The stdout should include '"stderr"'
  End

  rebuild_embedded_source() {
    mkdir -p "$WORKSPACE/src"
    cat > "$WORKSPACE/once.toml" <<'TOML'
[[target]]
name = "embedded"
kind = "rust_library"
srcs = ["src/**"]

[target.attrs]
crate_name = "embedded"

[[target]]
name = "app"
kind = "rust_binary"
srcs = ["main.rs"]
deps = ["./embedded"]

[target.attrs]
crate_root = "main.rs"
TOML
    cat > "$WORKSPACE/src/lib.rs" <<'RS'
pub fn message() -> &'static str { include_str!("message.txt") }
RS
    printf 'fn main() { print!("{}", embedded::message()); }\n' > "$WORKSPACE/main.rs"
    printf 'original payload\n' > "$WORKSPACE/src/message.txt"
    once build app > "$WORKSPACE/first-build.log" 2>&1 || { cat "$WORKSPACE/first-build.log" >&2; return 1; }
    "$WORKSPACE/.once/out/app/app" || return 1
    printf 'updated payload\n' > "$WORKSPACE/src/message.txt"
    once build app > "$WORKSPACE/second-build.log" 2>&1 || { cat "$WORKSPACE/second-build.log" >&2; return 1; }
    "$WORKSPACE/.once/out/app/app"
  }

  It 'rebuilds consumers when a non-Rust source embedded by a library changes'
    Skip if 'Rust toolchain is unavailable' rust_toolchain_unavailable

    When call rebuild_embedded_source
    The status should be success
    The line 1 of stdout should equal 'original payload'
    The line 2 of stdout should equal 'updated payload'
  End
End
