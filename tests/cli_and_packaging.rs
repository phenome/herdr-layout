use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = "usage: herdr-layout <1|2|3> | --dry-run <1|2|3> | --check-config <1|2|3>";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_herdr-layout"))
}

fn repo_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn temp_dir(name: &str) -> PathBuf {
    let mut path = env::temp_dir();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    path.push(format!("herdr-layout-{name}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn check_config_outputs_compact_json() {
    let dir = temp_dir("check-config");
    let config_dir = dir.join("config");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.yaml"),
        r#"
layouts:
  "1":
    tabs:
      - label: agent
        command: omp
      - label: git
        command: lazygit
"#,
    )
    .unwrap();

    let output = Command::new(bin())
        .arg("--check-config")
        .arg("1")
        .env("HERDR_PLUGIN_CONFIG_DIR", &config_dir)
        .env("HERDR_BIN_PATH", dir.join("missing-herdr"))
        .current_dir(&dir)
        .output()
        .unwrap();

    assert!(output.status.success(), "status: {:?}, stderr: {}", output.status.code(), String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let trimmed = stdout.trim_end_matches(['\r', '\n']);
    assert_eq!(stdout.matches('\n').count(), 1, "{stdout}");
    assert!(!trimmed.contains(' '), "{stdout}");
    assert_eq!(trimmed.matches("\":").count(), 4, "{stdout}");
    assert!(trimmed.contains("\"ok\":true"), "{stdout}");
    assert!(trimmed.contains("\"slot\":\"1\""), "{stdout}");
    assert!(trimmed.contains("\"targets\":2"), "{stdout}");
    assert!(trimmed.contains("\"files\":["), "{stdout}");
    assert_eq!(trimmed.matches("config.yaml").count(), 1, "{stdout}");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn invalid_args_exit_2() {
    let dir = temp_dir("invalid-args");
    let output = Command::new(bin())
        .env("HERDR_BIN_PATH", dir.join("missing-herdr"))
        .current_dir(&dir)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains(USAGE));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn manifest_actions_stay_on_cmd_wrapper() {
    let manifest = fs::read_to_string(repo_path("herdr-plugin.toml")).unwrap();

    for slot in ["1", "2", "3"] {
        assert!(manifest.contains(&format!("command = [\"bin/herdr-layout.cmd\", \"{slot}\"]")), "slot {slot}");
    }
    assert_eq!(manifest.matches("command = [\"bin/herdr-layout.cmd\", ").count(), 3);
}

#[test]
fn install_scripts_log_binary_download_and_keep_wrappers() {
    let ps1 = fs::read_to_string(repo_path("scripts/install.ps1")).unwrap();
    let sh = fs::read_to_string(repo_path("scripts/install.sh")).unwrap();

    assert!(ps1.contains("$ProgressPreference = \"SilentlyContinue\""));
    assert!(ps1.contains("Write-Host \"Downloading Herdr Layout binary: $Url\""));
    assert!(ps1.contains("Write-Host \"Installed Herdr Layout binary: $Out\""));
    assert!(ps1.contains("herdr-layout.cmd"));
    assert!(ps1.contains("if /I \"%~1\"==\"/herdr-layout.cmd\" ("));
    assert!(ps1.contains("%~dp0bin\\herdr-layout.exe"));
    assert!(sh.contains("echo \"Downloading Herdr Layout binary: $url\""));
    assert!(sh.contains("echo \"Installed Herdr Layout binary: $out\""));
    assert!(sh.contains("herdr-layout.cmd"));
    assert!(sh.contains(r#"exec "$(dirname "$0")/herdr-layout" "$@""#));
}
