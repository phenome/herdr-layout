use serde::Serialize;
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

pub type AppResult<T, E = String> = std::result::Result<T, E>;

type Slot = String;
type LayoutMap = HashMap<Slot, Layout>;

const USAGE: &str = "usage: herdr-layout <1|2|3> | --dry-run <1|2|3> | --check-config <1|2|3>";
const SHELLS: &[&str] = &["pwsh", "powershell", "cmd", "zsh", "bash", "fish", "nu", "sh"];
const HERDR_RETRIES: usize = 3;
const HERDR_RETRY_MS: u64 = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LayoutTarget {
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub first_uses_current_tab: bool,
    pub tabs: Vec<LayoutTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    pub label: String,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneInfo {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub focused: bool,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    pub agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneProcess {
    pub name: String,
    pub argv0: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneProcessInfo {
    pub pane_id: String,
    pub foreground_processes: Vec<PaneProcess>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub tabs: Vec<TabInfo>,
    pub panes: Vec<PaneInfo>,
    pub processes: HashMap<String, PaneProcessInfo>,
    pub current_pane: Option<PaneInfo>,
    pub original_tab_id: Option<String>,
    pub workspace_id: Option<String>,
    pub active_cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Operation {
    #[serde(rename_all = "camelCase")]
    AlreadyRunning { label: String, pane_id: String, tab_id: String },
    #[serde(rename_all = "camelCase")]
    RunExisting { target: LayoutTarget, pane_id: String, tab_id: String },
    #[serde(rename_all = "camelCase")]
    RenameCurrent { target: LayoutTarget, pane_id: String, tab_id: String },
    #[serde(rename_all = "camelCase")]
    ClaimCurrent { target: LayoutTarget, pane_id: String, tab_id: String },
    CreateTab { target: LayoutTarget },
}

#[derive(Clone, Debug)]
struct LoadedConfig {
    global_path: String,
    override_path: Option<String>,
    layouts: LayoutMap,
}

#[derive(Serialize)]
struct CheckConfigOutput {
    ok: bool,
    slot: Slot,
    targets: usize,
    files: Vec<String>,
}

#[derive(Serialize)]
struct DryRunOutput {
    ok: bool,
    slot: Slot,
    files: Vec<String>,
    operations: Vec<Operation>,
}

#[derive(Clone, Debug, Default)]
struct PluginContext {
    workspace_id: Option<String>,
    tab_id: Option<String>,
    focused_pane_id: Option<String>,
    focused_pane_cwd: Option<String>,
    workspace_cwd: Option<String>,
}

enum Mode {
    Apply(Slot),
    DryRun(Slot),
    CheckConfig(Slot),
}

fn main() {
    std::process::exit(run(env::args().skip(1).collect()));
}

fn run(argv: Vec<String>) -> i32 {
    let Some(mode) = parse_args(&argv) else {
        notify_user(USAGE);
        return 2;
    };

    let context = read_plugin_context();
    let invoking_cwd = context
        .focused_pane_cwd
        .as_deref()
        .or(context.workspace_cwd.as_deref())
        .map(str::to_string)
        .unwrap_or_else(process_cwd);

    if let Mode::CheckConfig(slot) = mode {
        return match load_config(&invoking_cwd).and_then(|loaded| {
            let layout = select_layout(&loaded.layouts, &HashMap::new(), &slot)?;
            Ok((loaded, layout))
        }) {
            Ok((loaded, layout)) => {
                let out = CheckConfigOutput { ok: true, slot, targets: layout.tabs.len(), files: config_files(&loaded) };
                println!("{}", serde_json::to_string(&out).expect("check-config JSON serializes"));
                0
            }
            Err(error) => {
                notify_user(&error);
                1
            }
        };
    }

    let slot = match &mode {
        Mode::Apply(slot) | Mode::DryRun(slot) | Mode::CheckConfig(slot) => slot.clone(),
    };
    let mode_name = if matches!(mode, Mode::DryRun(_)) { "dry-run" } else { "apply" };
    log_line(&format!("{mode_name} slot {slot}; cwd {invoking_cwd}"));

    let (loaded, layout) = match load_config(&invoking_cwd).and_then(|loaded| {
        let layout = select_layout(&loaded.layouts, &HashMap::new(), &slot)?;
        Ok((loaded, layout))
    }) {
        Ok(value) => value,
        Err(error) => {
            notify_user(&error);
            return 1;
        }
    };
    log_line(&format!("loaded {} target(s) from {}", layout.tabs.len(), config_files(&loaded).join(", ")));

    let snapshot = match read_snapshot(&context) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            notify_user(&error);
            return 1;
        }
    };

    let operations = match plan_layout(&layout, &snapshot) {
        Ok(operations) => operations,
        Err(error) => {
            notify_user(&error);
            return 1;
        }
    };
    log_line(&format!("planned {} operation(s): {:?}", operations.len(), operations));

    if matches!(mode, Mode::DryRun(_)) {
        let out = DryRunOutput { ok: true, slot, files: config_files(&loaded), operations };
        println!("{}", serde_json::to_string_pretty(&out).expect("dry-run JSON serializes"));
        return 0;
    }

    match apply_operations(&operations, &snapshot).and_then(|_| {
        if let Some(tab_id) = &snapshot.original_tab_id {
            herdr_ok(&["tab", "focus", tab_id])?;
        }
        Ok(())
    }) {
        Ok(()) => {
            notify_user(&format!("Applied layout {slot}: {} operation(s)", operations.len()));
            0
        },
        Err(error) => {
            notify_user(&error);
            1
        }
    }
}

fn parse_args(argv: &[String]) -> Option<Mode> {
    match argv {
        [slot] => parse_slot(Some(slot)).map(Mode::Apply),
        [flag, slot] if flag == "--dry-run" => parse_slot(Some(slot)).map(Mode::DryRun),
        [flag, slot] if flag == "--check-config" => parse_slot(Some(slot)).map(Mode::CheckConfig),
        _ => None,
    }
}

pub fn parse_slot(value: Option<&String>) -> Option<Slot> {
    match value.map(String::as_str) {
        Some("1" | "2" | "3") => value.cloned(),
        _ => None,
    }
}

fn load_config(invoking_cwd: &str) -> AppResult<LoadedConfig> {
    let config_dir = env::var("HERDR_PLUGIN_CONFIG_DIR").map_err(|_| "HERDR_PLUGIN_CONFIG_DIR is not set".to_string())?;
    let global = Path::new(&config_dir).join("config.yaml");
    if !global.exists() {
        return Err(format!("{}: config.yaml missing", global.display()));
    }

    let global_path = path_string(&global);
    let mut layouts = read_config_file(&global_path)?;
    let override_path = find_repo_override(invoking_cwd)?;
    if let Some(path) = &override_path {
        layouts.extend(read_config_file(path)?);
    }
    Ok(LoadedConfig { global_path, override_path, layouts })
}

pub fn read_config_file(path: &str) -> AppResult<LayoutMap> {
    let text = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let raw = serde_yaml::from_str::<YamlValue>(&text).map_err(|error| format!("{path}: {error}"))?;
    validate_config_root(&raw, path)
}

pub fn validate_config_root(raw: &YamlValue, source: &str) -> AppResult<LayoutMap> {
    let root = raw.as_mapping().ok_or_else(|| format!("{source}: YAML root must be a map"))?;
    let layouts_key = YamlValue::String("layouts".to_string());
    let layouts = root
        .get(&layouts_key)
        .and_then(YamlValue::as_mapping)
        .ok_or_else(|| format!("{source}: layouts must be a map"))?;

    let mut out = HashMap::new();
    for (raw_slot, raw_layout) in layouts {
        let raw_slot = raw_slot.as_str().map(str::to_string).unwrap_or_else(|| yaml_key(raw_slot));
        let slot = match raw_slot.as_str() {
            "1" | "2" | "3" => raw_slot.clone(),
            _ => return Err(format!("{source}: invalid layout slot {raw_slot}")),
        };
        out.insert(slot, validate_layout(raw_layout, &format!("{source}: layouts.{raw_slot}"))?);
    }
    Ok(out)
}

pub fn select_layout(global_layouts: &LayoutMap, override_layouts: &LayoutMap, slot: &str) -> AppResult<Layout> {
    override_layouts
        .get(slot)
        .or_else(|| global_layouts.get(slot))
        .cloned()
        .ok_or_else(|| format!("Layout slot {slot} is missing or invalid"))
}

fn validate_layout(raw: &YamlValue, path: &str) -> AppResult<Layout> {
    let layout = raw.as_mapping().ok_or_else(|| format!("{path} must be a map"))?;
    let first = match layout.get(YamlValue::String("firstUsesCurrentTab".to_string())) {
        Some(YamlValue::Bool(value)) => *value,
        Some(_) => return Err(format!("{path}.firstUsesCurrentTab must be boolean")),
        None => false,
    };
    let tabs_value = layout.get(YamlValue::String("tabs".to_string()));
    let tabs_raw = tabs_value
        .and_then(YamlValue::as_sequence)
        .filter(|tabs| !tabs.is_empty())
        .ok_or_else(|| format!("{path}.tabs must be a non-empty list"))?;

    let mut seen = HashSet::new();
    let mut tabs = Vec::with_capacity(tabs_raw.len());
    for (index, target) in tabs_raw.iter().enumerate() {
        let target = validate_target(target, &format!("{path}.tabs[{index}]"))?;
        if !seen.insert(target.label.clone()) {
            return Err(format!("{path}: duplicate tab label {}", target.label));
        }
        tabs.push(target);
    }
    Ok(Layout { first_uses_current_tab: first, tabs })
}

fn validate_target(raw: &YamlValue, path: &str) -> AppResult<LayoutTarget> {
    let target = raw.as_mapping().ok_or_else(|| format!("{path} must be a map"))?;
    let label = yaml_string(target.get(YamlValue::String("label".to_string())))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{path}.label must be a non-empty string"))?;
    let command = yaml_string(target.get(YamlValue::String("command".to_string())))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{path}.command must be a non-empty string"))?;
    Ok(LayoutTarget { label, command })
}

fn yaml_string(value: Option<&YamlValue>) -> Option<String> {
    value.and_then(YamlValue::as_str).map(str::to_string)
}

fn yaml_key(value: &YamlValue) -> String {
    match value {
        YamlValue::Number(n) => n.to_string(),
        YamlValue::Bool(v) => v.to_string(),
        _ => format!("{value:?}"),
    }
}

pub fn find_repo_override(start: &str) -> AppResult<Option<String>> {
    let mut dir = absolute_path(start);
    loop {
        let yaml = dir.join(".herdr-layout.yaml");
        let yml = dir.join(".herdr-layout.yml");
        let has_yaml = yaml.exists();
        let has_yml = yml.exists();
        if has_yaml && has_yml {
            return Err(format!("{}: both .herdr-layout.yaml and .herdr-layout.yml exist", dir.display()));
        }
        if has_yaml {
            return Ok(Some(path_string(&yaml)));
        }
        if has_yml {
            return Ok(Some(path_string(&yml)));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

fn read_snapshot(context: &PluginContext) -> AppResult<Snapshot> {
    let mut workspace_arg = Vec::new();
    if let Some(workspace_id) = &context.workspace_id {
        workspace_arg.push("--workspace".to_string());
        workspace_arg.push(workspace_id.clone());
    }

    let mut tab_args = vec!["tab".to_string(), "list".to_string()];
    tab_args.extend(workspace_arg.clone());
    let mut pane_args = vec!["pane".to_string(), "list".to_string()];
    pane_args.extend(workspace_arg);

    let tabs = parse_tab_list(&herdr_json_owned(&tab_args)?)?;
    let panes = parse_pane_list(&herdr_json_owned(&pane_args)?)?;
    if tabs.is_empty() {
        return Err("No Herdr tabs found".to_string());
    }
    if panes.is_empty() {
        return Err("No Herdr panes found".to_string());
    }

    let mut processes = HashMap::new();
    for pane in &panes {
        let args = vec!["pane".to_string(), "process-info".to_string(), "--pane".to_string(), pane.pane_id.clone()];
        let info = parse_process_info(&herdr_json_owned(&args)?)?;
        processes.insert(pane.pane_id.clone(), info);
    }

    let current_pane = panes
        .iter()
        .find(|pane| Some(pane.pane_id.as_str()) == context.focused_pane_id.as_deref())
        .or_else(|| panes.iter().find(|pane| pane.focused))
        .cloned();
    let original_tab_id = context
        .tab_id
        .clone()
        .or_else(|| tabs.iter().find(|tab| tab.focused).map(|tab| tab.tab_id.clone()))
        .or_else(|| current_pane.as_ref().map(|pane| pane.tab_id.clone()));
    let active_cwd = context
        .focused_pane_cwd
        .clone()
        .or_else(|| current_pane.as_ref().and_then(|pane| pane.foreground_cwd.clone()))
        .or_else(|| current_pane.as_ref().and_then(|pane| pane.cwd.clone()))
        .unwrap_or_else(process_cwd);
    let workspace_id = context
        .workspace_id
        .clone()
        .or_else(|| current_pane.as_ref().map(|pane| pane.workspace_id.clone()))
        .or_else(|| {
            original_tab_id
                .as_deref()
                .and_then(|id| tabs.iter().find(|tab| tab.tab_id == id).map(|tab| tab.workspace_id.clone()))
        });

    Ok(Snapshot { tabs, panes, processes, current_pane, original_tab_id, workspace_id, active_cwd })
}

pub fn plan_layout(layout: &Layout, snapshot: &Snapshot) -> AppResult<Vec<Operation>> {
    let mut tabs = snapshot.tabs.clone();
    let mut assigned = HashSet::new();
    let mut operations = Vec::new();

    for (index, target) in layout.tabs.iter().enumerate() {
        if index == 0 && layout.first_uses_current_tab {
            if let Some(current) = &snapshot.current_pane {
                if is_target_running(current, target, &snapshot.processes) {
                    assigned.insert(current.pane_id.clone());
                    if let Some(tab) = tabs.iter_mut().find(|tab| tab.tab_id == current.tab_id) {
                        tab.label.clone_from(&target.label);
                    }
                    operations.push(Operation::RenameCurrent { target: target.clone(), pane_id: current.pane_id.clone(), tab_id: current.tab_id.clone() });
                    continue;
                }

                if is_idle(current, &snapshot.processes) {
                    let planned = plan_target(target, &tabs, &snapshot.panes, &snapshot.processes, &mut assigned)?;
                    if !matches!(planned, Operation::CreateTab { .. }) {
                        operations.push(planned);
                        continue;
                    }

                    assigned.insert(current.pane_id.clone());
                    if let Some(tab) = tabs.iter_mut().find(|tab| tab.tab_id == current.tab_id) {
                        tab.label.clone_from(&target.label);
                    }
                    operations.push(Operation::ClaimCurrent { target: target.clone(), pane_id: current.pane_id.clone(), tab_id: current.tab_id.clone() });
                    continue;
                }
            }
        }
        operations.push(plan_target(target, &tabs, &snapshot.panes, &snapshot.processes, &mut assigned)?);
    }

    Ok(operations)
}

pub fn plan_target(
    target: &LayoutTarget,
    tabs: &[TabInfo],
    panes: &[PaneInfo],
    processes: &HashMap<String, PaneProcessInfo>,
    assigned: &mut HashSet<String>,
) -> AppResult<Operation> {
    let matching_tabs: Vec<&TabInfo> = tabs.iter().filter(|tab| tab.label == target.label).collect();
    if matching_tabs.is_empty() {
        return Ok(Operation::CreateTab { target: target.clone() });
    }

    for tab in &matching_tabs {
        if let Some(pane) = panes.iter().find(|pane| pane.tab_id == tab.tab_id && !assigned.contains(&pane.pane_id) && is_target_running(pane, target, processes)) {
            assigned.insert(pane.pane_id.clone());
            return Ok(Operation::AlreadyRunning { label: target.label.clone(), pane_id: pane.pane_id.clone(), tab_id: tab.tab_id.clone() });
        }
    }

    for tab in &matching_tabs {
        if let Some(pane) = panes.iter().find(|pane| pane.tab_id == tab.tab_id && !assigned.contains(&pane.pane_id)) {
            assigned.insert(pane.pane_id.clone());
            return Ok(Operation::AlreadyRunning { label: target.label.clone(), pane_id: pane.pane_id.clone(), tab_id: tab.tab_id.clone() });
        }
    }

    for tab in &matching_tabs {
        if let Some(pane) = panes.iter().find(|pane| pane.tab_id == tab.tab_id && !assigned.contains(&pane.pane_id) && is_idle(pane, processes)) {
            assigned.insert(pane.pane_id.clone());
            return Ok(Operation::RunExisting { target: target.clone(), pane_id: pane.pane_id.clone(), tab_id: tab.tab_id.clone() });
        }
    }

    Err(format!("Tab \"{}\" exists, but no matching or idle pane is available", target.label))
}

fn apply_operations(operations: &[Operation], snapshot: &Snapshot) -> AppResult<()> {
    for operation in operations {
        match operation {
            Operation::AlreadyRunning { .. } => {}
            Operation::RenameCurrent { target, tab_id, .. } => herdr_ok(&["tab", "rename", tab_id, &target.label])?,
            Operation::ClaimCurrent { target, pane_id, tab_id } => {
                herdr_ok(&["tab", "rename", tab_id, &target.label])?;
                herdr_ok(&["pane", "run", pane_id, &target.command])?;
            }
            Operation::RunExisting { target, pane_id, .. } => herdr_ok(&["pane", "run", pane_id, &target.command])?,
            Operation::CreateTab { target } => {
                let mut args = vec!["tab", "create"];
                if let Some(workspace_id) = &snapshot.workspace_id {
                    args.push("--workspace");
                    args.push(workspace_id);
                }
                args.extend(["--cwd", &snapshot.active_cwd, "--label", &target.label, "--no-focus"]);
                let root_pane = parse_created_root_pane(&herdr_json(&args)?)?;
                herdr_ok(&["pane", "run", &root_pane.pane_id, &target.command])?;
            }
        }
    }
    Ok(())
}

pub fn is_target_running(pane: &PaneInfo, target: &LayoutTarget, processes: &HashMap<String, PaneProcessInfo>) -> bool {
    let target_name = command_name(&target.command);
    if normalize_name(pane.agent.as_deref()) == target_name {
        return true;
    }
    processes.get(&pane.pane_id).is_some_and(|info| {
        info.foreground_processes
            .iter()
            .any(|process| normalize_name(process.argv0.as_deref().or(Some(&process.name))) == target_name)
    })
}

pub fn is_idle(pane: &PaneInfo, processes: &HashMap<String, PaneProcessInfo>) -> bool {
    let foreground = processes.get(&pane.pane_id).map(|info| info.foreground_processes.as_slice()).unwrap_or(&[]);
    !foreground.is_empty()
        && foreground
            .iter()
            .all(|process| SHELLS.contains(&normalize_name(process.argv0.as_deref().or(Some(&process.name))).as_str()))
}

pub fn command_name(command: &str) -> String {
    let trimmed = command.trim_start();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut chars = trimmed.chars();
    let quote = chars.next().unwrap_or_default();
    if quote == '\'' || quote == '"' {
        let rest = &trimmed[quote.len_utf8()..];
        let end = rest.find(quote).unwrap_or(rest.len());
        return normalize_name(Some(&rest[..end]));
    }
    normalize_name(trimmed.split_whitespace().next())
}

pub fn normalize_name(value: Option<&str>) -> String {
    let Some(value) = value.filter(|value| !value.is_empty()) else { return String::new(); };
    let leaf = value
        .trim_matches(|ch| ch == '\'' || ch == '"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value);
    let lower = leaf.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

pub fn herdr_json(args: &[&str]) -> AppResult<JsonValue> {
    let stdout = herdr(args, true)?;
    serde_json::from_str(&stdout).map_err(|error| error.to_string())
}

fn herdr_json_owned(args: &[String]) -> AppResult<JsonValue> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    herdr_json(&refs)
}

pub fn herdr_ok(args: &[&str]) -> AppResult<()> {
    herdr(args, false).map(|_| ())
}

fn herdr(args: &[&str], expect_json: bool) -> AppResult<String> {
    let bin = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_string());
    let command = format!("herdr {}", args.join(" "));
    for attempt in 0..=HERDR_RETRIES {
        let output = match Command::new(&bin).args(args).output() {
            Ok(output) => output,
            Err(error) => {
                let message = format!("{command} failed to start: {error}");
                if attempt < HERDR_RETRIES && message.contains("BrokenPipe") {
                    thread::sleep(Duration::from_millis(HERDR_RETRY_MS));
                    continue;
                }
                log_line(&message);
                return Err(message);
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if should_retry_herdr_output(output.status.success(), expect_json, &stdout, &stderr) && attempt < HERDR_RETRIES {
            thread::sleep(Duration::from_millis(HERDR_RETRY_MS));
            continue;
        }
        if !output.status.success() {
            let fallback = format!("{command} failed with exit {}", output.status.code().unwrap_or(-1));
            let detail = if !stderr.is_empty() { stderr } else if !stdout.is_empty() { stdout } else { fallback };
            let message = format!("{command}: {}", detail.trim());
            log_line(&message);
            return Err(message);
        }
        if expect_json && stdout.trim().is_empty() {
            let message = format!("{command} returned empty output");
            log_line(&message);
            return Err(message);
        }
        return Ok(stdout);
    }
    unreachable!("herdr retry loop returns")
}

fn should_retry_herdr_output(status_success: bool, expect_json: bool, stdout: &str, stderr: &str) -> bool {
    (!status_success && (stderr.contains("BrokenPipe") || stdout.contains("BrokenPipe") || stderr.contains("code: 232") || stdout.contains("code: 232")))
        || (expect_json && status_success && stdout.trim().is_empty())
}

fn log_line(message: &str) {
    eprintln!("[herdr-layout] {message}");
}

pub fn notify_user(message: &str) {
    log_line(message);
    let _ = herdr_ok(&["notification", "show", "Herdr Layout", "--body", message, "--position", "top-right", "--sound", "request"]);
}

fn parse_tab_list(response: &JsonValue) -> AppResult<Vec<TabInfo>> {
    let result = expect_result(response, "tab_list")?;
    let tabs = result.get("tabs").and_then(JsonValue::as_array).ok_or_else(|| "herdr tab list: tabs missing".to_string())?;
    tabs.iter().map(parse_tab_info).collect()
}

fn parse_pane_list(response: &JsonValue) -> AppResult<Vec<PaneInfo>> {
    let result = expect_result(response, "pane_list")?;
    let panes = result.get("panes").and_then(JsonValue::as_array).ok_or_else(|| "herdr pane list: panes missing".to_string())?;
    panes.iter().map(parse_pane_info).collect()
}

fn parse_process_info(response: &JsonValue) -> AppResult<PaneProcessInfo> {
    let result = expect_result(response, "pane_process_info")?;
    let info = result
        .get("process_info")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "herdr pane process-info: process_info missing".to_string())?;
    let foreground = info.get("foreground_processes").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    Ok(PaneProcessInfo {
        pane_id: read_string_obj(info, "pane_id", "process_info")?,
        foreground_processes: foreground.iter().map(parse_pane_process).collect::<AppResult<_>>()?,
    })
}

fn parse_created_root_pane(response: &JsonValue) -> AppResult<PaneInfo> {
    let result = expect_result(response, "tab_created")?;
    let root = result.get("root_pane").ok_or_else(|| "herdr tab create: root_pane missing".to_string())?;
    parse_pane_info(root)
}

fn parse_tab_info(raw: &JsonValue) -> AppResult<TabInfo> {
    let raw = raw.as_object().ok_or_else(|| "tab info must be object".to_string())?;
    Ok(TabInfo {
        tab_id: read_string_obj(raw, "tab_id", "tab")?,
        workspace_id: read_string_obj(raw, "workspace_id", "tab")?,
        label: read_string_obj(raw, "label", "tab")?,
        focused: raw.get("focused").and_then(JsonValue::as_bool) == Some(true),
    })
}

fn parse_pane_info(raw: &JsonValue) -> AppResult<PaneInfo> {
    let raw = raw.as_object().ok_or_else(|| "pane info must be object".to_string())?;
    Ok(PaneInfo {
        pane_id: read_string_obj(raw, "pane_id", "pane")?,
        workspace_id: read_string_obj(raw, "workspace_id", "pane")?,
        tab_id: read_string_obj(raw, "tab_id", "pane")?,
        focused: raw.get("focused").and_then(JsonValue::as_bool) == Some(true),
        cwd: optional_json_string(raw.get("cwd")),
        foreground_cwd: optional_json_string(raw.get("foreground_cwd")),
        agent: optional_json_string(raw.get("agent")),
    })
}

fn parse_pane_process(raw: &JsonValue) -> AppResult<PaneProcess> {
    let raw = raw.as_object().ok_or_else(|| "process info must be object".to_string())?;
    Ok(PaneProcess { name: read_string_obj(raw, "name", "process")?, argv0: optional_json_string(raw.get("argv0")) })
}

fn expect_result<'a>(response: &'a JsonValue, result_type: &str) -> AppResult<&'a serde_json::Map<String, JsonValue>> {
    let response = response.as_object().ok_or_else(|| "Herdr response must be object".to_string())?;
    if let Some(error) = response.get("error").and_then(JsonValue::as_object) {
        return Err(optional_json_string(error.get("message")).unwrap_or_else(|| JsonValue::Object(error.clone()).to_string()));
    }
    let result = response
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Herdr response missing result".to_string())?;
    let found = result.get("type").and_then(JsonValue::as_str).unwrap_or_default();
    if found != result_type {
        return Err(format!("Expected Herdr result {result_type}, got {found}"));
    }
    Ok(result)
}

fn read_plugin_context() -> PluginContext {
    let parsed = parse_plugin_context();
    PluginContext {
        workspace_id: pick_string(&parsed, &["workspace_id"])
            .or_else(|| nested_string(&parsed, "workspace", &["workspace_id", "id"]))
            .or_else(|| env_string("HERDR_WORKSPACE_ID")),
        tab_id: pick_string(&parsed, &["tab_id"])
            .or_else(|| nested_string(&parsed, "tab", &["tab_id", "id"]))
            .or_else(|| env_string("HERDR_TAB_ID")),
        focused_pane_id: pick_string(&parsed, &["focused_pane_id", "pane_id"])
            .or_else(|| nested_string(&parsed, "focused_pane", &["pane_id", "id"]))
            .or_else(|| env_string("HERDR_PANE_ID")),
        focused_pane_cwd: pick_string(&parsed, &["focused_pane_cwd"])
            .or_else(|| nested_string(&parsed, "focused_pane", &["foreground_cwd", "cwd"])),
        workspace_cwd: pick_string(&parsed, &["workspace_cwd"]).or_else(|| nested_string(&parsed, "workspace", &["cwd"])),
    }
}

pub fn parse_plugin_context() -> JsonValue {
    env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .ok()
        .and_then(|raw| serde_json::from_str::<JsonValue>(&raw).ok())
        .filter(JsonValue::is_object)
        .unwrap_or_else(|| JsonValue::Object(Default::default()))
}

fn pick_string(record: &JsonValue, keys: &[&str]) -> Option<String> {
    let object = record.as_object()?;
    keys.iter().find_map(|key| optional_json_string(object.get(*key)))
}

fn nested_string(record: &JsonValue, key: &str, keys: &[&str]) -> Option<String> {
    pick_string(record.get(key)?, keys)
}

fn read_string_obj(record: &serde_json::Map<String, JsonValue>, key: &str, owner: &str) -> AppResult<String> {
    record
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{owner}.{key} must be string"))
}

fn optional_json_string(value: Option<&JsonValue>) -> Option<String> {
    value.and_then(JsonValue::as_str).filter(|value| !value.is_empty()).map(str::to_string)
}

fn env_string(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.is_empty())
}

fn config_files(loaded: &LoadedConfig) -> Vec<String> {
    let mut files = vec![loaded.global_path.clone()];
    if let Some(path) = &loaded.override_path {
        files.push(path.clone());
    }
    files
}

fn absolute_path(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn process_cwd() -> String {
    env::current_dir().map(|path| path_string(&path)).unwrap_or_else(|_| ".".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(label: &str, command: &str) -> LayoutTarget {
        LayoutTarget { label: label.to_string(), command: command.to_string() }
    }

    fn layout(tabs: Vec<LayoutTarget>) -> Layout {
        Layout { first_uses_current_tab: false, tabs }
    }

    fn tab(tab_id: &str, label: &str) -> TabInfo {
        TabInfo { tab_id: tab_id.to_string(), workspace_id: "w".to_string(), label: label.to_string(), focused: false }
    }

    fn pane(pane_id: &str, tab_id: &str) -> PaneInfo {
        PaneInfo {
            pane_id: pane_id.to_string(),
            workspace_id: "w".to_string(),
            tab_id: tab_id.to_string(),
            focused: false,
            cwd: None,
            foreground_cwd: None,
            agent: None,
        }
    }

    fn pane_with_agent(pane_id: &str, tab_id: &str, agent: &str) -> PaneInfo {
        PaneInfo { agent: Some(agent.to_string()), ..pane(pane_id, tab_id) }
    }

    fn process(name: &str) -> PaneProcess {
        PaneProcess { name: name.to_string(), argv0: None }
    }

    fn process_with_argv0(name: &str, argv0: &str) -> PaneProcess {
        PaneProcess { name: name.to_string(), argv0: Some(argv0.to_string()) }
    }

    fn processes(entries: &[(&str, Vec<PaneProcess>)]) -> HashMap<String, PaneProcessInfo> {
        entries
            .iter()
            .map(|(pane_id, foreground_processes)| {
                (
                    (*pane_id).to_string(),
                    PaneProcessInfo { pane_id: (*pane_id).to_string(), foreground_processes: foreground_processes.clone() },
                )
            })
            .collect()
    }

    fn snapshot(
        tabs: Vec<TabInfo>,
        panes: Vec<PaneInfo>,
        process_map: HashMap<String, PaneProcessInfo>,
        current_pane: Option<PaneInfo>,
    ) -> Snapshot {
        Snapshot {
            tabs,
            panes,
            processes: process_map,
            current_pane,
            original_tab_id: None,
            workspace_id: None,
            active_cwd: "/repo".to_string(),
        }
    }

    fn yaml(raw: &str) -> YamlValue {
        serde_yaml::from_str(raw).expect("test YAML parses")
    }

    #[test]
    fn config_validation_accepts_slots_1_2_3() {
        let raw = yaml(
            r#"
layouts:
  "1":
    tabs:
      - label: one
        command: one
  "2":
    tabs:
      - label: two
        command: two
  "3":
    tabs:
      - label: three
        command: three
"#,
        );
        let got = validate_config_root(&raw, "cfg").unwrap();
        let expected = HashMap::from([
            ("1".to_string(), layout(vec![target("one", "one")])),
            ("2".to_string(), layout(vec![target("two", "two")])),
            ("3".to_string(), layout(vec![target("three", "three")])),
        ]);

        assert_eq!(got, expected);
    }

    #[test]
    fn config_validation_rejects_invalid_shapes_fields_and_duplicate_labels() {
        let cases = [
            ("root", "null", "YAML root must be a map"),
            ("layouts", "{}", "layouts must be a map"),
            (
                "slot",
                r#"
layouts:
  "4":
    tabs:
      - label: api
        command: api
"#,
                "invalid layout slot 4",
            ),
            (
                "tabs",
                r#"
layouts:
  "1":
    tabs: []
"#,
                "tabs must be a non-empty list",
            ),
            (
                "label",
                r#"
layouts:
  "1":
    tabs:
      - label: ""
        command: api
"#,
                "label must be a non-empty string",
            ),
            (
                "command",
                r#"
layouts:
  "1":
    tabs:
      - label: api
        command: ""
"#,
                "command must be a non-empty string",
            ),
            (
                "duplicate",
                r#"
layouts:
  "1":
    tabs:
      - label: api
        command: api
      - label: api
        command: worker
"#,
                "duplicate tab label api",
            ),
        ];

        for (name, raw, fragment) in cases {
            let error = validate_config_root(&yaml(raw), &format!("cfg-{name}")).unwrap_err();
            assert!(error.contains(fragment), "{name}: {error}");
        }
    }

    #[test]
    fn repo_override_replaces_whole_slot_layout() {
        let global = HashMap::from([("1".to_string(), layout(vec![target("global-a", "global-a"), target("global-b", "global-b")]))]);
        let override_layout = Layout { first_uses_current_tab: true, tabs: vec![target("repo-only", "repo-only")] };
        let overrides = HashMap::from([("1".to_string(), override_layout.clone())]);

        assert_eq!(select_layout(&global, &overrides, "1").unwrap(), override_layout);
    }

    #[test]
    fn planning_matches_tab_labels_exactly() {
        let desired = target("api", "api --serve");

        assert_eq!(
            plan_layout(
                &layout(vec![desired.clone()]),
                &snapshot(vec![tab("t1", "API")], vec![pane("p1", "t1")], processes(&[("p1", vec![process("bash")])]), None),
            )
            .unwrap(),
            vec![Operation::CreateTab { target: desired }],
        );
    }

    #[test]
    fn target_running_matches_command_basename_exe_and_pane_agent() {
        assert!(is_target_running(
            &pane("p1", "t1"),
            &target("api", r"C:\tools\api.exe --serve"),
            &processes(&[("p1", vec![process_with_argv0("ignored", "/usr/local/bin/api.exe")])]),
        ));

        assert!(is_target_running(&pane_with_agent("p2", "t2", "api.exe"), &target("api", "api --serve"), &HashMap::new()));
    }

    #[test]
    fn idle_panes_require_known_foreground_shells() {
        for shell in ["pwsh", "powershell.exe", "cmd", "zsh", "bash", "fish", "nu", "sh"] {
            let pane_id = format!("p-{shell}");
            assert!(is_idle(&pane(&pane_id, "t1"), &processes(&[(&pane_id, vec![process(shell)])])), "{shell}");
        }

        assert!(!is_idle(&pane("p-node", "t1"), &processes(&[("p-node", vec![process("node")])])));
        assert!(!is_idle(&pane("p-empty", "t1"), &processes(&[("p-empty", vec![])])));
    }

    #[test]
    fn first_uses_current_tab_prefers_already_running_target_elsewhere() {
        let desired = target("api", "api.exe --serve");
        let current = pane("p-current", "t-current");
        let mut first = layout(vec![desired]);
        first.first_uses_current_tab = true;

        assert_eq!(
            plan_layout(
                &first,
                &snapshot(
                    vec![tab("t-current", "scratch"), tab("t-existing", "api")],
                    vec![current.clone(), pane("p-existing", "t-existing")],
                    processes(&[("p-current", vec![process("pwsh")]), ("p-existing", vec![process("api.exe")])]),
                    Some(current),
                ),
            )
            .unwrap(),
            vec![Operation::AlreadyRunning { label: "api".to_string(), pane_id: "p-existing".to_string(), tab_id: "t-existing".to_string() }],
        );
    }

    #[test]
    fn first_uses_current_tab_renames_current_running_target() {
        let desired = target("agent", "omp");
        let current = pane("p-current", "t-current");
        let mut first = layout(vec![desired.clone()]);
        first.first_uses_current_tab = true;

        assert_eq!(
            plan_layout(
                &first,
                &snapshot(
                    vec![tab("t-current", "1")],
                    vec![current.clone()],
                    processes(&[("p-current", vec![process("omp")])]),
                    Some(current),
                ),
            )
            .unwrap(),
            vec![Operation::RenameCurrent { target: desired, pane_id: "p-current".to_string(), tab_id: "t-current".to_string() }],
        );
    }

    #[test]
    fn matching_label_shell_foreground_is_noop_to_avoid_tui_keystrokes() {
        let desired = target("files", "yazi");
        let tabs = vec![tab("t-files", "files")];
        let panes = vec![pane("p-files", "t-files")];

        assert_eq!(
            plan_layout(
                &layout(vec![desired]),
                &snapshot(tabs, panes, processes(&[("p-files", vec![process("pwsh")])]), None),
            )
            .unwrap(),
            vec![Operation::AlreadyRunning { label: "files".to_string(), pane_id: "p-files".to_string(), tab_id: "t-files".to_string() }],
        );
    }

    #[test]
    fn duplicate_target_tabs_prefer_running_then_idle_then_error() {
        let desired = target("api", "api --serve");
        let tabs = vec![tab("t-idle", "api"), tab("t-running", "api")];
        let panes = vec![pane("p-idle", "t-idle"), pane("p-running", "t-running")];

        assert_eq!(
            plan_layout(
                &layout(vec![desired.clone()]),
                &snapshot(
                    tabs.clone(),
                    panes.clone(),
                    processes(&[("p-idle", vec![process("bash")]), ("p-running", vec![process("api")])]),
                    None,
                ),
            )
            .unwrap(),
            vec![Operation::AlreadyRunning { label: "api".to_string(), pane_id: "p-running".to_string(), tab_id: "t-running".to_string() }],
        );

        assert_eq!(

            plan_layout(
                &layout(vec![desired.clone()]),
                &snapshot(
                    tabs.clone(),
                    panes.clone(),
                    processes(&[("p-idle", vec![process("bash")]), ("p-running", vec![process("zsh")])]),
                    None,
                ),
            )
            .unwrap(),
            vec![Operation::AlreadyRunning { label: "api".to_string(), pane_id: "p-idle".to_string(), tab_id: "t-idle".to_string() }],
        );

        assert_eq!(
            plan_layout(
                &layout(vec![desired]),
                &snapshot(tabs, panes, processes(&[("p-idle", vec![process("node")]), ("p-running", vec![process("python")])]), None),
            )
            .unwrap(),
            vec![Operation::AlreadyRunning { label: "api".to_string(), pane_id: "p-idle".to_string(), tab_id: "t-idle".to_string() }],
        );
    }

    #[test]
    fn herdr_retry_detects_pipe_failures_and_empty_json() {
        assert!(should_retry_herdr_output(false, false, "", r#"Error: Os { code: 232, kind: BrokenPipe, message: "pipe closing" }"#));
        assert!(should_retry_herdr_output(false, false, "BrokenPipe", ""));
        assert!(should_retry_herdr_output(true, true, "", ""));
        assert!(!should_retry_herdr_output(true, true, r#"{"ok":true}"#, ""));
        assert!(!should_retry_herdr_output(false, false, "", "real error"));
    }
}
