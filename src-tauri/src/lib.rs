use chrono::Local;
use rusqlite::{params, Connection, DatabaseName, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItemBuilder, MenuItemKind, SubmenuBuilder},
    Manager,
};
#[cfg(target_os = "macos")]
use tauri::menu::{AboutMetadata, PredefinedMenuItem};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    roots: Vec<String>,
    session_count: usize,
    issue_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    id: String,
    title: String,
    project_id: Option<String>,
    project_name: Option<String>,
    project_roots: Vec<String>,
    database_cwd: String,
    conversation_cwd: Option<String>,
    log_path: Option<String>,
    archived: bool,
    status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrphanRecord {
    id: String,
    conversation_cwd: Option<String>,
    log_path: String,
    archived: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    projects: usize,
    sessions: usize,
    matched: usize,
    mismatched: usize,
    unlinked: usize,
    missing_logs: usize,
    orphan_records: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditReport {
    codex_home: String,
    state_database: String,
    default_backup_directory: String,
    projects: Vec<Project>,
    sessions: Vec<Session>,
    orphans: Vec<OrphanRecord>,
    summary: Summary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepairRequest {
    thread_id: String,
    target_path: String,
    confirmation: String,
    backup_base: Option<String>,
    #[serde(default)]
    include_child_agents: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteRequest {
    log_path: String,
    confirmation: String,
    backup_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteSessionRequest {
    thread_id: String,
    confirmation: String,
    backup_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollbackRequest {
    manifest_path: String,
    backup_base: Option<String>,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteRollbackRequest {
    manifest_path: String,
    backup_base: Option<String>,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupDeleteRequest {
    backup_folder: String,
    backup_base: Option<String>,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClearHistoryRequest {
    kind: String,
    backup_base: Option<String>,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportSessionsRequest {
    thread_ids: Vec<String>,
    destination_directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportSessionsRequest {
    manifest_path: String,
    confirmation: String,
    backup_base: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportProject {
    id: String,
    name: String,
    roots: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportLog {
    thread_id: String,
    file: String,
    archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionExportManifest {
    format: String,
    version: u32,
    exported_at: String,
    projects: Vec<ExportProject>,
    visible_thread_ids: Vec<String>,
    thread_ids: Vec<String>,
    #[serde(default)]
    project_assignments: HashMap<String, String>,
    logs: Vec<ExportLog>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportPackageInfo {
    manifest_path: String,
    exported_at: String,
    projects: Vec<ExportProject>,
    visible_thread_count: usize,
    thread_count: usize,
    log_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionResult {
    message: String,
    backup_folder: String,
    changes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessCloseResult {
    message: String,
    requested: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupFileEntry {
    original_path: String,
    backup_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairManifest {
    version: u32,
    created_at: String,
    thread_id: String,
    session_title: String,
    source_cwd: String,
    target_cwd: String,
    files: Vec<BackupFileEntry>,
    #[serde(default)]
    thread_changes: Vec<ThreadRepairChange>,
    rolled_back_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadRepairChange {
    thread_id: String,
    session_title: String,
    source_cwd: String,
    target_cwd: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepairHistoryItem {
    created_at: String,
    thread_id: String,
    session_title: String,
    source_cwd: String,
    target_cwd: String,
    backup_folder: String,
    manifest_path: String,
    file_count: usize,
    rolled_back_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteManifest {
    version: u32,
    created_at: String,
    completed_at: Option<String>,
    deletion_kind: String,
    thread_id: Option<String>,
    session_title: String,
    source_path: Option<String>,
    child_count: usize,
    files: Vec<BackupFileEntry>,
    #[serde(default)]
    rolled_back_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteHistoryItem {
    created_at: String,
    deletion_kind: String,
    thread_id: Option<String>,
    session_title: String,
    source_path: Option<String>,
    child_count: usize,
    backup_folder: String,
    file_count: usize,
    rolled_back_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupHistoryItem {
    folder: String,
    name: String,
    operation: String,
    created_at: String,
    file_count: usize,
    total_size: u64,
}

struct ZoomLevel(Mutex<f64>);

const DEFAULT_ZOOM: f64 = 1.0;
const ZOOM_STEP: f64 = 0.1;
const MIN_ZOOM: f64 = 0.5;
const MAX_ZOOM: f64 = 3.0;

#[derive(Clone, Debug)]
struct DbThread {
    id: String,
    title: String,
    source: String,
    cwd: String,
    archived: bool,
    rollout_path: String,
    project_id: Option<String>,
}

#[derive(Clone, Debug)]
struct RepairTarget {
    id: String,
    title: String,
    cwd: String,
    logs: Vec<LogMeta>,
}

#[derive(Clone, Debug)]
struct LogMeta {
    id: String,
    cwd: Option<String>,
    path: PathBuf,
    archived: bool,
}

fn codex_home() -> Result<PathBuf, String> {
    let path = if let Some(configured) = std::env::var_os("CODEX_HOME") {
        PathBuf::from(configured)
    } else {
        platform_home_directory()
            .ok_or("无法读取用户主目录（HOME 或 USERPROFILE）")?
            .join(".codex")
    };
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("未找到 Codex 数据目录：{}", path.display()))
    }
}

fn platform_home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn state_database(home: &Path) -> Result<PathBuf, String> {
    for candidate in [home.join("state_5.sqlite"), home.join("sqlite/state_5.sqlite")] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("未找到 state_5.sqlite；请确认 Codex Desktop 已至少启动过一次。".into())
}

fn desktop_catalog_database(home: &Path) -> PathBuf {
    home.join("sqlite").join("codex-dev.db")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn normalized_for_platform(path: &str, windows: bool) -> String {
    let mut value = path.trim().replace('\\', "/");
    if windows {
        if let Some(path) = value.strip_prefix("//?/UNC/") {
            value = format!("//{path}");
        } else if let Some(path) = value.strip_prefix("//?/") {
            value = path.to_string();
        }
    }
    while value.len() > 1
        && value.ends_with('/')
        && !(windows && value.len() == 3 && value.as_bytes().get(1) == Some(&b':'))
    {
        value.pop();
    }
    if windows { value.to_lowercase() } else { value }
}

fn normalized(path: &str) -> String {
    normalized_for_platform(path, cfg!(windows))
}

fn is_same_path(left: &str, right: &str) -> bool {
    normalized(left) == normalized(right)
}

fn is_managed_log(home: &Path, path: &Path) -> bool {
    let in_sessions = path.starts_with(home.join("sessions"));
    let in_archived = path.starts_with(home.join("archived_sessions"));
    (in_sessions || in_archived) && path.extension().is_some_and(|ext| ext == "jsonl")
}

fn query_projects(conn: &Connection) -> Result<Vec<Project>, String> {
    let mut roots_by_project: HashMap<String, Vec<String>> = HashMap::new();
    let mut root_stmt = conn
        .prepare("SELECT project_id, path FROM project_roots ORDER BY project_id, position")
        .map_err(|e| format!("无法读取 Codex 项目路径：{e}"))?;
    let root_rows = root_stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for root in root_rows {
        let (id, path) = root.map_err(|e| e.to_string())?;
        roots_by_project.entry(id).or_default().push(path);
    }

    let mut stmt = conn
        .prepare("SELECT id, name FROM projects ORDER BY position, id")
        .map_err(|e| format!("无法读取 Codex 项目：{e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut projects = Vec::new();
    for row in rows {
        let (id, name) = row.map_err(|e| e.to_string())?;
        projects.push(Project {
            roots: roots_by_project.remove(&id).unwrap_or_default(),
            id,
            name,
            session_count: 0,
            issue_count: 0,
        });
    }
    Ok(projects)
}

fn query_threads(conn: &Connection) -> Result<Vec<DbThread>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, COALESCE(NULLIF(TRIM(name), ''), title), source, cwd, archived, rollout_path, project_id
             FROM threads ORDER BY recency_at_ms DESC, id DESC",
        )
        .map_err(|e| format!("无法读取 Codex 会话：{e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DbThread {
                id: row.get(0)?,
                title: row.get(1)?,
                source: row.get(2)?,
                cwd: row.get(3)?,
                archived: row.get::<_, i64>(4)? != 0,
                rollout_path: row.get(5)?,
                project_id: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.map(|row| row.map_err(|e| e.to_string())).collect()
}

fn descendant_thread_ids(conn: &Connection, root_thread_id: &str) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut seen = HashSet::from([root_thread_id.to_string()]);
    let mut pending = VecDeque::from([root_thread_id.to_string()]);
    let mut statement = conn
        .prepare("SELECT child_thread_id FROM thread_spawn_edges WHERE parent_thread_id = ?1 ORDER BY child_thread_id")
        .map_err(|e| format!("无法读取子代理关系：{e}"))?;

    while let Some(parent_id) = pending.pop_front() {
        let rows = statement
            .query_map(params![parent_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("无法读取子代理关系：{e}"))?;
        for row in rows {
            let child_id = row.map_err(|e| format!("无法读取子代理关系：{e}"))?;
            if seen.insert(child_id.clone()) {
                pending.push_back(child_id.clone());
                ids.push(child_id);
            }
        }
    }
    Ok(ids)
}

fn load_repair_targets(
    conn: &Connection,
    root_thread_id: &str,
    include_child_agents: bool,
    logs_by_id: &HashMap<String, Vec<LogMeta>>,
) -> Result<Vec<RepairTarget>, String> {
    let mut ids = vec![root_thread_id.to_string()];
    if include_child_agents {
        ids.extend(descendant_thread_ids(conn, root_thread_id)?);
    }

    let mut statement = conn
        .prepare(
            "SELECT id, cwd, COALESCE(NULLIF(TRIM(name), ''), title)
             FROM threads WHERE id = ?1",
        )
        .map_err(|e| format!("无法读取目标会话：{e}"))?;
    let mut targets = Vec::new();
    for id in ids {
        let (id, cwd, title) = statement
            .query_row(params![id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(|e| format!("未找到{}：{e}", if id == root_thread_id { "目标会话" } else { "子代理会话" }))?;
        let logs = logs_by_id.get(&id).cloned().unwrap_or_default();
        if logs.is_empty() {
            return Err(format!("找不到{}“{title}”的 JSONL 日志，已拒绝修改任何会话。", if id == root_thread_id { "目标会话" } else { "子代理会话" }));
        }
        targets.push(RepairTarget { id, title, cwd, logs });
    }
    Ok(targets)
}

fn parse_log(path: &Path, archived: bool) -> Option<LogMeta> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(80).flatten() {
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload")?;
        let id = payload
            .get("id")
            .or_else(|| payload.get("session_id"))
            .and_then(Value::as_str)?
            .to_string();
        let cwd = payload.get("cwd").and_then(Value::as_str).map(str::to_string);
        return Some(LogMeta { id, cwd, path: path.to_path_buf(), archived });
    }
    None
}

fn scan_logs(home: &Path) -> Vec<LogMeta> {
    let mut logs = Vec::new();
    for (folder, archived) in [(home.join("sessions"), false), (home.join("archived_sessions"), true)] {
        if !folder.is_dir() { continue; }
        for entry in WalkDir::new(folder).follow_links(false).into_iter().flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                if let Some(log) = parse_log(path, archived) { logs.push(log); }
            }
        }
    }
    logs
}

fn safe_package_file(package_root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute() || path.components().any(|part| !matches!(part, std::path::Component::Normal(_))) {
        return Err("导出包包含无效的文件路径。".into());
    }
    let resolved = package_root.join(path);
    if !resolved.starts_with(package_root) { return Err("导出包包含越界的文件路径。".into()); }
    Ok(resolved)
}

fn safe_imported_thread_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn read_export_manifest(path: &Path) -> Result<SessionExportManifest, String> {
    if path.file_name().and_then(|name| name.to_str()) != Some("manifest.json") {
        return Err("请选择导出包中的 manifest.json。".into());
    }
    let content = fs::read_to_string(path).map_err(|e| format!("无法读取导出包清单：{e}"))?;
    let manifest: SessionExportManifest = serde_json::from_str(&content)
        .map_err(|e| format!("导出包清单格式无效：{e}"))?;
    if manifest.format != "codex-session-manager-export" || manifest.version != 1 {
        return Err("这不是受支持的 Codex 会话导出包。".into());
    }
    if manifest.thread_ids.is_empty() || manifest.visible_thread_ids.is_empty() {
        return Err("导出包不包含可导入的会话。".into());
    }
    let ids = manifest.thread_ids.iter().collect::<HashSet<_>>();
    if ids.len() != manifest.thread_ids.len() || manifest.thread_ids.iter().any(|id| !safe_imported_thread_id(id)) {
        return Err("导出包包含无效或重复的会话 ID。".into());
    }
    if manifest.visible_thread_ids.iter().any(|id| !ids.contains(id)) {
        return Err("导出包中的主会话范围无效。".into());
    }
    Ok(manifest)
}

fn trim_export_database(snapshot: &Path, thread_ids: &[String], project_ids: &HashSet<String>) -> Result<(), String> {
    let mut conn = Connection::open(snapshot).map_err(|e| format!("无法整理导出状态数据库：{e}"))?;
    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA secure_delete = ON; CREATE TEMP TABLE exported_thread_ids (id TEXT PRIMARY KEY); CREATE TEMP TABLE exported_project_ids (id TEXT PRIMARY KEY);")
        .map_err(|e| format!("无法准备导出状态数据库：{e}"))?;
    {
        let transaction = conn.transaction().map_err(|e| format!("无法整理导出状态数据库：{e}"))?;
        for id in thread_ids {
            transaction.execute("INSERT INTO exported_thread_ids (id) VALUES (?1)", params![id])
                .map_err(|e| format!("无法整理导出会话范围：{e}"))?;
        }
        for id in project_ids {
            transaction.execute("INSERT INTO exported_project_ids (id) VALUES (?1)", params![id])
                .map_err(|e| format!("无法整理导出项目范围：{e}"))?;
        }
        transaction.execute("DELETE FROM thread_spawn_edges WHERE parent_thread_id NOT IN (SELECT id FROM exported_thread_ids) OR child_thread_id NOT IN (SELECT id FROM exported_thread_ids)", [])
            .map_err(|e| format!("无法裁剪子代理关系：{e}"))?;
        transaction.execute("DELETE FROM thread_dynamic_tools WHERE thread_id NOT IN (SELECT id FROM exported_thread_ids)", [])
            .map_err(|e| format!("无法裁剪会话工具配置：{e}"))?;
        transaction.execute("DELETE FROM thread_artifacts WHERE thread_id NOT IN (SELECT id FROM exported_thread_ids)", [])
            .map_err(|e| format!("无法裁剪会话产物配置：{e}"))?;
        transaction.execute("DELETE FROM threads WHERE id NOT IN (SELECT id FROM exported_thread_ids)", [])
            .map_err(|e| format!("无法裁剪会话记录：{e}"))?;
        transaction.execute("DELETE FROM thread_sections WHERE id NOT IN (SELECT DISTINCT thread_section_id FROM threads WHERE thread_section_id IS NOT NULL)", [])
            .map_err(|e| format!("无法裁剪会话分组配置：{e}"))?;
        transaction.execute("DELETE FROM project_roots WHERE project_id NOT IN (SELECT DISTINCT project_id FROM threads WHERE project_id IS NOT NULL) AND project_id NOT IN (SELECT id FROM exported_project_ids)", [])
            .map_err(|e| format!("无法裁剪项目目录配置：{e}"))?;
        transaction.execute("DELETE FROM projects WHERE id NOT IN (SELECT DISTINCT project_id FROM threads WHERE project_id IS NOT NULL) AND id NOT IN (SELECT id FROM exported_project_ids)", [])
            .map_err(|e| format!("无法裁剪项目配置：{e}"))?;
        transaction.commit().map_err(|e| format!("无法提交导出状态裁剪：{e}"))?;
    }
    for table in [
        "_sqlx_migrations", "backfill_state", "external_agent_config_imports", "project_idempotency_keys",
        "remote_control_enrollments", "rollout_migration_skipped_rollouts", "rollout_migration_state",
    ] {
        conn.execute(&format!("DROP TABLE IF EXISTS {table}"), [])
            .map_err(|e| format!("无法清理导出状态数据库：{e}"))?;
    }
    conn.execute_batch("VACUUM").map_err(|e| format!("无法压缩导出状态数据库：{e}"))?;
    Ok(())
}

fn imported_log_destination(home: &Path, batch: &str, thread_id: &str, index: usize) -> PathBuf {
    home.join("sessions").join("imported").join(batch).join(format!("{thread_id}-{index}.jsonl"))
}

fn update_global_state_assignments(home: &Path, assignments: &HashMap<String, Option<String>>) -> Result<usize, String> {
    let path = home.join(".codex-global-state.json");
    let mut root = if path.is_file() {
        let content = fs::read_to_string(&path).map_err(|e| format!("无法读取 Codex 侧栏状态：{e}"))?;
        serde_json::from_str::<Value>(&content).map_err(|e| format!("Codex 侧栏状态格式无效：{e}"))?
    } else {
        Value::Object(serde_json::Map::new())
    };
    let root_object = root.as_object_mut().ok_or("Codex 侧栏状态格式无效。")?;
    let assignments_value = root_object.entry("thread-project-assignments")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let assignments_object = assignments_value.as_object_mut().ok_or("Codex 项目归属状态格式无效。")?;
    let mut updated = 0;
    for (thread_id, project_id) in assignments {
        let Some(project_id) = project_id else { continue };
        assignments_object.insert(thread_id.clone(), serde_json::json!({
            "projectKind": "local",
            "projectId": project_id,
        }));
        updated += 1;
    }
    atomic_write(&path, &serde_json::to_string(&root).map_err(|e| format!("无法生成 Codex 侧栏状态：{e}"))?)?;
    Ok(updated)
}

/// Codex Desktop keeps its sidebar project membership separately from the
/// app-server thread table. This preserves membership when a project root is
/// changed, while the conversation itself still has the previous cwd.
fn load_desktop_project_assignments(home: &Path) -> HashMap<String, String> {
    let state_path = home.join(".codex-global-state.json");
    let Ok(raw) = fs::read(&state_path) else { return HashMap::new() };
    let Ok(root) = serde_json::from_slice::<Value>(&raw) else { return HashMap::new() };
    let Some(assignments) = root.get("thread-project-assignments").and_then(Value::as_object) else {
        return HashMap::new();
    };
    let legacy_mappings = root
        .get("app-server-project-id-by-legacy-project-id-by-host")
        .and_then(Value::as_object);
    let mut result = HashMap::new();
    for (thread_id, assignment) in assignments {
        if assignment.get("projectKind").and_then(Value::as_str) != Some("local") { continue; }
        let Some(legacy_id) = assignment.get("projectId").and_then(Value::as_str) else { continue; };
        let current_id = legacy_mappings
            .and_then(|hosts| hosts.values().find_map(|projects| projects.get(legacy_id)))
            .and_then(Value::as_str)
            .unwrap_or(legacy_id);
        result.insert(thread_id.clone(), current_id.to_string());
    }
    result
}

fn effective_project_id(
    thread: &DbThread,
    projects: &[Project],
    desktop_assignments: &HashMap<String, String>,
) -> Option<String> {
    thread.project_id.clone()
        .or_else(|| desktop_assignments.get(&thread.id).cloned())
        .or_else(|| {
            projects
                .iter()
                .find(|project| project.roots.iter().any(|root| is_same_path(root, &thread.cwd)))
                .map(|project| project.id.clone())
        })
}

fn report() -> Result<AuditReport, String> {
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("无法打开 Codex 状态数据库：{e}"))?;
    let mut projects = query_projects(&conn)?;
    let all_threads = query_threads(&conn)?;
    let desktop_assignments = load_desktop_project_assignments(&home);
    let logs = scan_logs(&home);
    let mut logs_by_id: HashMap<String, Vec<LogMeta>> = HashMap::new();
    for log in logs.iter().cloned() { logs_by_id.entry(log.id.clone()).or_default().push(log); }

    let project_lookup: HashMap<String, (String, Vec<String>)> = projects
        .iter()
        .map(|p| (p.id.clone(), (p.name.clone(), p.roots.clone())))
        .collect();
    // `source = vscode` is the set that Codex Desktop presents as normal user
    // conversations. Guardian-review and spawned-agent rollouts are persisted in
    // the same table, but are not sidebar sessions and must not inflate a project's
    // displayed count. Keep their IDs below for the history/orphan comparison.
    let thread_ids: HashSet<String> = all_threads.iter().map(|thread| thread.id.clone()).collect();
    let mut sessions = Vec::new();
    let mut matched = 0;
    let mut mismatched = 0;
    let mut unlinked = 0;
    let mut missing_logs = 0;

    for thread in all_threads.into_iter().filter(|thread| thread.source == "vscode") {
        // Prefer Codex Desktop's persistent project assignment. It survives a root
        // migration, which lets the UI show the old cwd as a repairable mismatch.
        let effective_project_id = effective_project_id(&thread, &projects, &desktop_assignments);
        let project = effective_project_id.as_ref().and_then(|id| project_lookup.get(id)).cloned();
        let matching_log = logs_by_id
            .get(&thread.id)
            .and_then(|items| items.iter().find(|log| display_path(&log.path) == thread.rollout_path).or_else(|| items.first()));
        let conversation_cwd = matching_log.and_then(|log| log.cwd.clone());
        let log_path = matching_log.map(|log| display_path(&log.path));
        let status = if conversation_cwd.is_none() {
            missing_logs += 1;
            "missing_log"
        } else if project.is_none() {
            unlinked += 1;
            "unlinked"
        } else {
            let roots = &project.as_ref().unwrap().1;
            let same = is_same_path(&thread.cwd, conversation_cwd.as_deref().unwrap());
            let project_matches = roots.iter().any(|root| is_same_path(root, &thread.cwd));
            if same && project_matches {
                matched += 1;
                "match"
            } else {
                mismatched += 1;
                "mismatch"
            }
        }.to_string();
        if let Some(project_id) = &effective_project_id {
            if let Some(project) = projects.iter_mut().find(|project| &project.id == project_id) {
                project.session_count += 1;
                if status == "mismatch" || status == "missing_log" { project.issue_count += 1; }
            }
        }
        sessions.push(Session {
            id: thread.id,
            title: thread.title,
            project_id: effective_project_id,
            project_name: project.as_ref().map(|item| item.0.clone()),
            project_roots: project.map(|item| item.1).unwrap_or_default(),
            database_cwd: thread.cwd,
            conversation_cwd,
            log_path,
            archived: thread.archived,
            status,
        });
    }

    let mut seen_paths = HashSet::new();
    let orphans = logs
        .into_iter()
        .filter(|log| !thread_ids.contains(&log.id) && seen_paths.insert(display_path(&log.path)))
        .map(|log| OrphanRecord { id: log.id, conversation_cwd: log.cwd, log_path: display_path(&log.path), archived: log.archived })
        .collect::<Vec<_>>();

    Ok(AuditReport {
        codex_home: display_path(&home),
        state_database: display_path(&db_path),
        default_backup_directory: display_path(&default_backup_base(&home)),
        summary: Summary {
            projects: projects.len(), sessions: sessions.len(), matched, mismatched, unlinked, missing_logs,
            orphan_records: orphans.len(),
        },
        projects,
        sessions,
        orphans,
    })
}

fn default_backup_base(home: &Path) -> PathBuf {
    home.join("session-manager-backups")
}

fn backup_base(home: &Path, requested: Option<&str>) -> Result<PathBuf, String> {
    let requested = requested.map(str::trim).filter(|value| !value.is_empty());
    let base = requested.map(PathBuf::from).unwrap_or_else(|| default_backup_base(home));
    if !base.is_absolute() { return Err("备份目录必须是绝对路径。".into()); }
    if base.exists() && !base.is_dir() { return Err("备份位置不是目录。".into()); }
    fs::create_dir_all(&base).map_err(|e| format!("无法创建备份目录：{e}"))?;
    Ok(base)
}

fn backup_folder(home: &Path, operation: &str, requested_base: Option<&str>) -> Result<PathBuf, String> {
    let stamp = Local::now().format("%Y%m%d-%H%M%S-%3f");
    let path = backup_base(home, requested_base)?.join(format!("{stamp}-{operation}"));
    fs::create_dir_all(&path).map_err(|e| format!("无法创建备份目录：{e}"))?;
    Ok(path)
}

fn backup_file(source: &Path, destination: &Path, name: &str) -> Result<(), String> {
    if source.is_file() {
        fs::copy(source, destination.join(name)).map_err(|e| format!("备份 {} 失败：{e}", source.display()))?;
    }
    Ok(())
}

fn backup_database(db_path: &Path, destination: &Path) -> Result<(), String> {
    let source = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("无法打开要备份的状态数据库：{e}"))?;
    source
        .backup(DatabaseName::Main, destination.join("state_5.sqlite"), None)
        .map_err(|e| format!("创建一致性状态数据库备份失败：{e}"))
}

fn backup_entry(source: &Path, destination: &Path, name: &str) -> Result<Option<BackupFileEntry>, String> {
    if !source.is_file() { return Ok(None); }
    backup_file(source, destination, name)?;
    Ok(Some(BackupFileEntry {
        original_path: display_path(source),
        backup_name: name.to_string(),
    }))
}

fn write_manifest(folder: &Path, manifest: &RepairManifest) -> Result<PathBuf, String> {
    let path = folder.join("repair-history.json");
    let content = serde_json::to_string_pretty(manifest).map_err(|e| format!("无法生成修复历史：{e}"))?;
    fs::write(&path, content).map_err(|e| format!("无法写入修复历史：{e}"))?;
    Ok(path)
}

fn read_manifest(path: &Path) -> Result<RepairManifest, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("无法读取修复历史：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("修复历史格式无效：{e}"))
}

fn write_delete_manifest(folder: &Path, manifest: &DeleteManifest) -> Result<PathBuf, String> {
    let path = folder.join("delete-history.json");
    let content = serde_json::to_string_pretty(manifest).map_err(|e| format!("无法生成删除历史：{e}"))?;
    atomic_write(&path, &content).map_err(|e| format!("无法写入删除历史：{e}"))?;
    Ok(path)
}

fn read_delete_manifest(path: &Path) -> Result<DeleteManifest, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("无法读取删除历史：{e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("删除历史格式无效：{e}"))
}

fn repair_history(home: &Path, requested_base: Option<&str>) -> Result<Vec<RepairHistoryItem>, String> {
    let base = backup_base(home, requested_base)?;
    let mut items = Vec::new();
    for entry in fs::read_dir(&base).map_err(|e| format!("无法读取备份目录：{e}"))?.flatten() {
        let folder = entry.path();
        let manifest_path = folder.join("repair-history.json");
        if !folder.is_dir() || !manifest_path.is_file() { continue; }
        let Ok(manifest) = read_manifest(&manifest_path) else { continue };
        items.push(RepairHistoryItem {
            created_at: manifest.created_at,
            thread_id: manifest.thread_id,
            session_title: manifest.session_title,
            source_cwd: manifest.source_cwd,
            target_cwd: manifest.target_cwd,
            backup_folder: display_path(&folder),
            manifest_path: display_path(&manifest_path),
            file_count: manifest.files.len(),
            rolled_back_at: manifest.rolled_back_at,
        });
    }
    items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(items)
}

fn delete_history(home: &Path, requested_base: Option<&str>) -> Result<Vec<DeleteHistoryItem>, String> {
    let base = backup_base(home, requested_base)?;
    let mut items = Vec::new();
    for entry in fs::read_dir(&base).map_err(|e| format!("无法读取备份目录：{e}"))?.flatten() {
        let folder = entry.path();
        let manifest_path = folder.join("delete-history.json");
        if !folder.is_dir() || !manifest_path.is_file() { continue; }
        let Ok(manifest) = read_delete_manifest(&manifest_path) else { continue };
        let Some(completed_at) = manifest.completed_at else { continue };
        items.push(DeleteHistoryItem {
            created_at: completed_at,
            deletion_kind: manifest.deletion_kind,
            thread_id: manifest.thread_id,
            session_title: manifest.session_title,
            source_path: manifest.source_path,
            child_count: manifest.child_count,
            backup_folder: display_path(&folder),
            file_count: manifest.files.len(),
            rolled_back_at: manifest.rolled_back_at,
        });
    }
    items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(items)
}

fn backup_history(home: &Path, requested_base: Option<&str>) -> Result<Vec<BackupHistoryItem>, String> {
    let base = backup_base(home, requested_base)?;
    let mut items = Vec::new();
    for entry in fs::read_dir(&base).map_err(|e| format!("无法读取备份目录：{e}"))?.flatten() {
        let folder = entry.path();
        if !folder.is_dir() { continue; }
        let mut file_count = 0;
        let mut total_size = 0;
        for file in WalkDir::new(&folder).follow_links(false).into_iter().flatten().filter(|entry| entry.file_type().is_file()) {
            file_count += 1;
            total_size += file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        }
        let name = folder.file_name().and_then(|part| part.to_str()).unwrap_or("未命名备份").to_string();
        let operation = if name.contains("deleted-session") { "删除会话" } else if name.contains("deleted-orphan") { "删除遗留日志" } else if name.contains("before-import") { "导入前快照" } else if name.contains("repair") { "修复" } else if name.contains("rollback") { "回退前快照" } else { "其他备份" }.to_string();
        let created_at = fs::metadata(&folder).and_then(|metadata| metadata.modified()).ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| chrono::DateTime::<chrono::Utc>::from_timestamp(time.as_secs() as i64, 0).map(|time| time.with_timezone(&Local).to_rfc3339()).unwrap_or_else(|| "未知".into()))
            .unwrap_or_else(|| "未知".into());
        items.push(BackupHistoryItem { folder: display_path(&folder), name, operation, created_at, file_count, total_size });
    }
    items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(items)
}

fn validate_manifest_path(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = fs::canonicalize(base).map_err(|e| format!("无法验证备份目录：{e}"))?;
    let path = fs::canonicalize(path).map_err(|e| format!("无法验证修复历史：{e}"))?;
    if path.file_name().and_then(|name| name.to_str()) != Some("repair-history.json") || !path.starts_with(&base) {
        return Err("修复历史不属于当前备份目录。".into());
    }
    Ok(path)
}

fn validate_delete_manifest_path(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = fs::canonicalize(base).map_err(|e| format!("无法验证备份目录：{e}"))?;
    let path = fs::canonicalize(path).map_err(|e| format!("无法验证删除历史：{e}"))?;
    if path.file_name().and_then(|name| name.to_str()) != Some("delete-history.json") || !path.starts_with(&base) {
        return Err("删除历史不属于当前备份目录。".into());
    }
    Ok(path)
}

fn validate_backup_folder(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let base = fs::canonicalize(base).map_err(|e| format!("无法验证备份目录：{e}"))?;
    let path = fs::canonicalize(path).map_err(|e| format!("无法验证备份文件：{e}"))?;
    if !path.is_dir() || path.parent() != Some(base.as_path()) {
        return Err("只能删除当前备份目录中的单次备份。".into());
    }
    Ok(path)
}

fn is_restorable_path(home: &Path, db_path: &Path, path: &Path) -> bool {
    path == db_path
        || path == PathBuf::from(format!("{}-wal", db_path.display()))
        || path == PathBuf::from(format!("{}-shm", db_path.display()))
        || is_managed_log(home, path)
}

fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or("日志文件名无效")?;
    let temp = path.with_file_name(format!(".{file_name}.session-manager-{}.tmp", std::process::id()));
    fs::write(&temp, content).map_err(|e| format!("写入临时日志失败：{e}"))?;
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("替换日志失败：{error}"));
    }
    Ok(())
}

fn purge_thread_reference(value: &mut Value, thread_id: &str) -> usize {
    match value {
        Value::Object(values) => {
            let encoded_thread_id = thread_id.replace('-', "%2D");
            let keys = values.iter()
                .filter(|(key, item)| {
                    key.contains(thread_id)
                        || key.contains(&encoded_thread_id)
                        || (key.contains("new-thread:") && item.as_str() == Some(thread_id))
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            let mut removed = keys.len();
            for key in keys { values.remove(&key); }
            for child in values.values_mut() { removed += purge_thread_reference(child, thread_id); }
            removed
        }
        Value::Array(values) => {
            let before = values.len();
            values.retain(|item| item.as_str() != Some(thread_id));
            let mut removed = before - values.len();
            for child in values.iter_mut() { removed += purge_thread_reference(child, thread_id); }
            removed
        }
        _ => 0,
    }
}

fn purge_global_state_thread_references(home: &Path, thread_ids: &HashSet<String>) -> Result<usize, String> {
    let path = home.join(".codex-global-state.json");
    if !path.is_file() { return Ok(0); }
    let content = fs::read_to_string(&path).map_err(|e| format!("无法读取 Codex 侧栏状态：{e}"))?;
    let mut state: Value = serde_json::from_str(&content).map_err(|e| format!("Codex 侧栏状态格式无效：{e}"))?;
    let removed = thread_ids.iter().map(|id| purge_thread_reference(&mut state, id)).sum();
    if removed > 0 {
        let content = serde_json::to_string(&state).map_err(|e| format!("无法生成更新后的 Codex 侧栏状态：{e}"))?;
        atomic_write(&path, &content)?;
    }
    Ok(removed)
}

fn purge_desktop_catalog_thread_references(home: &Path, thread_ids: &HashSet<String>) -> Result<usize, String> {
    let path = desktop_catalog_database(home);
    if !path.is_file() { return Ok(0); }
    let mut conn = Connection::open(&path).map_err(|e| format!("无法打开 Codex 桌面目录缓存：{e}"))?;
    let transaction = conn.transaction().map_err(|e| format!("无法开始清理 Codex 桌面目录缓存：{e}"))?;
    let mut removed = 0;
    for thread_id in thread_ids {
        for table in ["local_thread_catalog", "local_thread_catalog_scan_entries", "thread_timeline_ledger", "inbox_items"] {
            let exists: Option<String> = transaction.query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get(0),
            ).optional().map_err(|e| format!("无法检查 Codex 桌面目录缓存：{e}"))?;
            if exists.is_some() {
                removed += transaction.execute(&format!("DELETE FROM {table} WHERE thread_id = ?1"), params![thread_id])
                    .map_err(|e| format!("无法清理 Codex 桌面目录缓存：{e}"))?;
            }
        }
    }
    transaction.commit().map_err(|e| format!("无法提交 Codex 桌面目录缓存清理：{e}"))?;
    Ok(removed)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    let source = source.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let destination = destination.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let succeeded = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

fn replace_path_values(value: &mut Value, previous_paths: &HashSet<String>, target: &str) -> usize {
    match value {
        Value::Array(items) => items.iter_mut().map(|item| replace_path_values(item, previous_paths, target)).sum(),
        Value::Object(fields) => fields.iter_mut().map(|(key, item)| {
            if key == "cwd" {
                if let Value::String(text) = item {
                    if previous_paths.contains(text) {
                        *text = target.to_string();
                        return 1;
                    }
                }
            }
            replace_path_values(item, previous_paths, target)
        }).sum(),
        _ => 0,
    }
}

fn rewrite_log(path: &Path, previous_paths: &HashSet<String>, target: &str) -> Result<usize, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("无法读取会话日志：{e}"))?;
    let mut replacements = 0;
    let mut lines = Vec::new();
    for line in raw.lines() {
        let mut value = serde_json::from_str::<Value>(line).map_err(|e| format!("会话日志不是有效 JSONL：{e}"))?;
        replacements += replace_path_values(&mut value, previous_paths, target);
        lines.push(serde_json::to_string(&value).map_err(|e| e.to_string())?);
    }
    if replacements > 0 {
        atomic_write(path, &(lines.join("\n") + "\n"))?;
    }
    Ok(replacements)
}

fn is_codex_process(command: &str) -> bool {
    let command = command.trim().replace('\\', "/").to_lowercase();
    command.contains("/applications/chatgpt.app/contents/macos/chatgpt")
        || command.contains("/applications/codex.app/contents/macos/codex")
        || matches!(command.as_str(), "chatgpt.exe" | "codex.exe" | "chatgpt" | "codex")
        || command.ends_with("/chatgpt.exe")
        || command.ends_with("/codex.exe")
        || (command.contains("codex") && command.contains("app-server"))
}

#[cfg(not(windows))]
fn running_codex_processes() -> Result<Vec<(String, String)>, String> {
    let output = Command::new("ps").args(["-ax", "-o", "pid=,command="]).output()
        .map_err(|e| format!("无法检查 Codex 是否已退出：{e}"))?;
    if !output.status.success() {
        return Err("无法检查 Codex 是否已退出。".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let mut fields = line.trim().splitn(2, char::is_whitespace);
        let pid = fields.next()?.to_string();
        let command = fields.next()?.trim().to_string();
        is_codex_process(&command).then_some((pid, command))
    }).collect())
}

#[cfg(any(windows, test))]
fn parse_windows_tasklist_line(line: &str) -> Option<(String, String)> {
    let fields = line.trim().trim_matches('"').split("\",\"").collect::<Vec<_>>();
    let image_name = fields.first()?.trim();
    let pid = fields.get(1)?.trim();
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((pid.to_string(), image_name.to_string()))
}

#[cfg(windows)]
fn running_codex_processes() -> Result<Vec<(String, String)>, String> {
    let output = Command::new("tasklist").args(["/FO", "CSV", "/NH"]).output()
        .map_err(|e| format!("无法检查 Codex 是否已退出：{e}"))?;
    if !output.status.success() {
        return Err("无法检查 Codex 是否已退出。".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_windows_tasklist_line)
        .filter(|(_, image_name)| is_codex_process(image_name))
        .collect())
}

#[cfg(not(windows))]
fn request_process_close(pid: &str) -> bool {
    Command::new("kill").args(["-TERM", pid]).status().is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn request_process_close(pid: &str) -> bool {
    Command::new("taskkill").args(["/PID", pid]).status().is_ok_and(|status| status.success())
}

fn ensure_codex_is_closed() -> Result<(), String> {
    let running = running_codex_processes()
        .map_err(|error| format!("{error}，因此已拒绝修改。"))?;
    if !running.is_empty() {
        return Err(format!("检测到 {} 个 Codex 进程仍在运行。请先关闭它们，再重新执行当前操作。", running.len()));
    }
    Ok(())
}

fn codex_executable_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(configured) = std::env::var_os("CODEX_BIN") {
        candidates.push(PathBuf::from(configured));
    }

    #[cfg(target_os = "macos")]
    {
        candidates.extend([
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex"),
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex"),
        ]);
        if let Some(user_home) = platform_home_directory() {
            candidates.push(user_home.join("Applications/ChatGPT.app/Contents/Resources/codex"));
            candidates.push(user_home.join("Applications/Codex.app/Contents/Resources/codex"));
        }
    }

    #[cfg(windows)]
    {
        for base in [std::env::var_os("LOCALAPPDATA"), std::env::var_os("ProgramFiles")].into_iter().flatten() {
            let base = PathBuf::from(base);
            candidates.push(base.join("Programs/ChatGPT/resources/codex.exe"));
            candidates.push(base.join("Programs/Codex/resources/codex.exe"));
            candidates.push(base.join("ChatGPT/resources/codex.exe"));
            candidates.push(base.join("Codex/resources/codex.exe"));
        }
    }

    // Keep PATH as a last resort: a separately installed CLI can be older than
    // the app's bundled app-server protocol.
    candidates.push(PathBuf::from(if cfg!(windows) { "codex.exe" } else { "codex" }));
    candidates
}

#[cfg(test)]
fn parse_thread_delete_response(stdout: &str) -> Result<(), String> {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        if value.get("id").and_then(Value::as_i64) != Some(2) { continue; }
        if value.get("result").is_some() { return Ok(()); }
        let message = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Codex app-server 返回未知删除错误");
        return Err(message.to_string());
    }
    Err("Codex app-server 未返回会话删除结果。".into())
}

fn wait_for_app_server_response(reader: &mut BufReader<std::process::ChildStdout>, request_id: i64) -> Result<Value, String> {
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).map_err(|error| format!("无法读取 Codex app-server 响应：{error}"))?;
        if read == 0 { return Err(format!("Codex app-server 在响应请求 {request_id} 前已退出。")); }
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        if value.get("id").and_then(Value::as_i64) == Some(request_id) { return Ok(value); }
    }
}

fn delete_thread_with_codex(home: &Path, thread_id: &str) -> Result<(), String> {
    let initialize = serde_json::json!({
        "method": "initialize",
        "id": 1,
        "params": { "clientInfo": { "name": "codex-session-manager", "version": env!("CARGO_PKG_VERSION") } }
    });
    let delete = serde_json::json!({
        "method": "thread/delete",
        "id": 2,
        "params": { "threadId": thread_id }
    });
    let mut launch_errors = Vec::new();

    for executable in codex_executable_candidates() {
        if executable.components().count() > 1 && !executable.is_file() { continue; }
        let mut child = match Command::new(&executable)
            .args(["app-server", "--stdio"])
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                launch_errors.push(format!("{}：{error}", executable.display()));
                continue;
            }
        };

        let mut stdin = child.stdin.take().ok_or("无法连接 Codex app-server 标准输入")?;
        let stdout = child.stdout.take().ok_or("无法连接 Codex app-server 标准输出")?;
        let mut reader = BufReader::new(stdout);
        let write_result = writeln!(stdin, "{initialize}").and_then(|_| stdin.flush());
        if let Err(error) = write_result {
            let _ = child.kill();
            return Err(format!("无法向 Codex app-server 发送初始化请求：{error}"));
        }
        let initialize_response = wait_for_app_server_response(&mut reader, 1);
        if let Err(error) = initialize_response {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Codex app-server 初始化失败：{error}"));
        }
        let write_result = writeln!(stdin, "{delete}").and_then(|_| stdin.flush());
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("无法向 Codex app-server 发送删除请求：{error}"));
        }
        let response = wait_for_app_server_response(&mut reader, 2);
        let _ = child.kill();
        let _ = child.wait();
        let response = response?;
        if response.get("result").is_some() { return Ok(()); }
        let message = response
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Codex app-server 返回未知删除错误");
        return Err(message.to_string());
    }

    let details = if launch_errors.is_empty() { String::new() } else { format!("：{}", launch_errors.join("；")) };
    Err(format!("找不到可用的 Codex app-server{details}。请安装或更新 Codex Desktop，也可通过 CODEX_BIN 指定 codex 可执行文件。"))
}

#[tauri::command]
fn scan_codex() -> Result<AuditReport, String> { report() }

#[tauri::command]
fn inspect_export_package(manifest_path: String) -> Result<ImportPackageInfo, String> {
    let path = fs::canonicalize(Path::new(manifest_path.trim())).map_err(|e| format!("无法读取导出包：{e}"))?;
    let manifest = read_export_manifest(&path)?;
    let root = path.parent().ok_or("导出包路径无效。")?;
    let snapshot = root.join("state_5.sqlite");
    if !snapshot.is_file() { return Err("导出包缺少状态数据库快照。".into()); }
    for log in &manifest.logs {
        if !manifest.thread_ids.iter().any(|id| id == &log.thread_id) { return Err("导出包日志归属无效。".into()); }
        let file = safe_package_file(root, &log.file)?;
        if !file.is_file() { return Err(format!("导出包缺少会话日志：{}", log.file)); }
    }
    Ok(ImportPackageInfo {
        manifest_path: display_path(&path),
        exported_at: manifest.exported_at,
        projects: manifest.projects,
        visible_thread_count: manifest.visible_thread_ids.len(),
        thread_count: manifest.thread_ids.len(),
        log_count: manifest.logs.len(),
    })
}

#[tauri::command]
fn export_sessions(request: ExportSessionsRequest) -> Result<ActionResult, String> {
    if request.thread_ids.is_empty() { return Err("请至少选择一个会话。".into()); }
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let destination = PathBuf::from(request.destination_directory.trim());
    if !destination.is_absolute() || !destination.is_dir() { return Err("导出位置必须是一个已存在的绝对目录。".into()); }
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("无法读取 Codex 状态数据库：{e}"))?;
    let mut ids = HashSet::new();
    for id in &request.thread_ids {
        let source: String = conn.query_row("SELECT source FROM threads WHERE id = ?1", params![id], |row| row.get(0))
            .map_err(|_| format!("找不到会话 {id}；请重新扫描后再导出。"))?;
        if source != "vscode" { return Err("只能导出 Codex 侧栏中的普通会话。".into()); }
        ids.insert(id.clone());
        ids.extend(descendant_thread_ids(&conn, id)?);
    }
    let mut thread_ids = ids.into_iter().collect::<Vec<_>>();
    thread_ids.sort();
    let projects_for_assignment = query_projects(&conn)?;
    let desktop_assignments = load_desktop_project_assignments(&home);
    let threads_by_id = query_threads(&conn)?.into_iter().map(|thread| (thread.id.clone(), thread)).collect::<HashMap<_, _>>();
    let project_assignments = thread_ids.iter().filter_map(|id| {
        let thread = threads_by_id.get(id)?;
        effective_project_id(thread, &projects_for_assignment, &desktop_assignments).map(|project_id| (id.clone(), project_id))
    }).collect::<HashMap<_, _>>();
    let mut projects = Vec::new();
    let mut project_ids = HashSet::new();
    for id in &thread_ids {
        if let Some(project_id) = project_assignments.get(id) { project_ids.insert(project_id.clone()); }
    }
    for project_id in &project_ids {
        let (id, name): (String, String) = conn.query_row("SELECT id, name FROM projects WHERE id = ?1", params![&project_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("无法读取项目配置：{e}"))?;
        let mut statement = conn.prepare("SELECT path FROM project_roots WHERE project_id = ?1 ORDER BY position")
            .map_err(|e| format!("无法读取项目目录：{e}"))?;
        let roots = statement.query_map(params![&id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("无法读取项目目录：{e}"))?
            .collect::<Result<Vec<_>, _>>().map_err(|e| format!("无法读取项目目录：{e}"))?;
        projects.push(ExportProject { id, name, roots });
    }
    projects.sort_by(|left, right| left.name.cmp(&right.name));
    let logs = scan_logs(&home).into_iter().filter(|log| thread_ids.contains(&log.id)).collect::<Vec<_>>();
    for visible_id in &request.thread_ids {
        if !logs.iter().any(|log| &log.id == visible_id) { return Err(format!("会话 {visible_id} 缺少 JSONL 日志，无法创建完整导出包。")); }
    }
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let package = destination.join(format!("codex-session-export-{stamp}"));
    fs::create_dir(&package).map_err(|e| format!("无法创建导出包目录：{e}"))?;
    let logs_folder = package.join("logs");
    fs::create_dir(&logs_folder).map_err(|e| format!("无法创建导出日志目录：{e}"))?;
    backup_database(&db_path, &package)?;
    trim_export_database(&package.join("state_5.sqlite"), &thread_ids, &project_ids)?;
    let mut exported_logs = Vec::new();
    for (index, log) in logs.iter().enumerate() {
        let name = format!("{index}-{}.jsonl", log.id);
        fs::copy(&log.path, logs_folder.join(&name)).map_err(|e| format!("无法导出会话日志：{e}"))?;
        exported_logs.push(ExportLog { thread_id: log.id.clone(), file: format!("logs/{name}"), archived: log.archived });
    }
    let mut visible_thread_ids = request.thread_ids.clone();
    visible_thread_ids.sort();
    visible_thread_ids.dedup();
    let manifest = SessionExportManifest {
        format: "codex-session-manager-export".into(),
        version: 1,
        exported_at: Local::now().to_rfc3339(),
        projects,
        visible_thread_ids,
        thread_ids,
        project_assignments,
        logs: exported_logs,
    };
    let manifest_path = package.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).map_err(|e| format!("无法生成导出清单：{e}"))?)
        .map_err(|e| format!("无法写入导出清单：{e}"))?;
    Ok(ActionResult {
        message: "已导出会话包。导入时请选择包内的 manifest.json。".into(),
        backup_folder: display_path(&package),
        changes: vec![
            format!("导出项目配置：{} 个", manifest.projects.len()),
            format!("导出会话：{} 个主会话，{} 个会话记录", manifest.visible_thread_ids.len(), manifest.thread_ids.len()),
            format!("导出对话日志：{} 个", manifest.logs.len()),
        ],
    })
}

#[tauri::command]
fn import_sessions(request: ImportSessionsRequest) -> Result<ActionResult, String> {
    if request.confirmation != "IMPORT" { return Err("导入确认无效。".into()); }
    let manifest_path = fs::canonicalize(Path::new(request.manifest_path.trim())).map_err(|e| format!("无法读取导出包：{e}"))?;
    let manifest = read_export_manifest(&manifest_path)?;
    let package = manifest_path.parent().ok_or("导出包路径无效。")?;
    let package_db = package.join("state_5.sqlite");
    if !package_db.is_file() { return Err("导出包缺少状态数据库快照。".into()); }
    let package_conn = Connection::open_with_flags(&package_db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("无法打开导出包数据库：{e}"))?;
    let mut project_by_thread = HashMap::new();
    for id in &manifest.thread_ids {
        let stored_project_id: Option<String> = package_conn.query_row("SELECT project_id FROM threads WHERE id = ?1", params![id], |row| row.get(0))
            .map_err(|_| format!("导出包数据库中找不到会话 {id}。"))?;
        let project_id = stored_project_id.or_else(|| manifest.project_assignments.get(id).cloned());
        project_by_thread.insert(id.clone(), project_id);
    }
    let mut logs_by_thread: HashMap<String, Vec<(PathBuf, bool)>> = HashMap::new();
    for log in &manifest.logs {
        if !project_by_thread.contains_key(&log.thread_id) { return Err("导出包日志归属无效。".into()); }
        let source = safe_package_file(package, &log.file)?;
        if !source.is_file() { return Err(format!("导出包缺少会话日志：{}", log.file)); }
        let parsed = parse_log(&source, log.archived).ok_or_else(|| format!("导出包日志格式无效：{}", log.file))?;
        if parsed.id != log.thread_id { return Err(format!("导出包日志 ID 不匹配：{}", log.file)); }
        logs_by_thread.entry(log.thread_id.clone()).or_default().push((source, log.archived));
    }
    for visible_id in &manifest.visible_thread_ids {
        if !logs_by_thread.contains_key(visible_id) { return Err(format!("导出包中的主会话 {visible_id} 缺少日志。")); }
    }
    ensure_codex_is_closed()?;
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let mut conn = Connection::open(&db_path).map_err(|e| format!("无法打开 Codex 状态数据库：{e}"))?;
    for id in &manifest.thread_ids {
        let existing: Option<String> = conn.query_row("SELECT id FROM threads WHERE id = ?1", params![id], |row| row.get(0)).optional()
            .map_err(|e| format!("无法检查现有会话：{e}"))?;
        if existing.is_some() { return Err(format!("当前 Codex 已存在会话 {id}；为避免覆盖，已取消导入。")); }
    }
    let safety = backup_folder(&home, "before-import", request.backup_base.as_deref())?;
    backup_database(&db_path, &safety)?;
    let mut backup_files = vec![BackupFileEntry { original_path: display_path(&db_path), backup_name: "state_5.sqlite".into() }];
    if let Some(entry) = backup_entry(&home.join(".codex-global-state.json"), &safety, "codex-global-state.json")? { backup_files.push(entry); }
    let batch = Local::now().format("%Y%m%d-%H%M%S-%3f-import").to_string();
    let imported_logs_root = home.join("sessions").join("imported").join(&batch);
    fs::create_dir_all(&imported_logs_root).map_err(|e| format!("无法创建导入会话目录：{e}"))?;
    let mut rollout_paths: HashMap<String, String> = HashMap::new();
    let mut copied_logs = 0;
    for (thread_id, logs) in &logs_by_thread {
        for (index, (source, _)) in logs.iter().enumerate() {
            let destination = imported_log_destination(&home, &batch, thread_id, index);
            fs::copy(source, &destination).map_err(|e| format!("无法写入导入会话日志：{e}"))?;
            rollout_paths.entry(thread_id.clone()).or_insert_with(|| display_path(&destination));
            copied_logs += 1;
        }
    }
    let result = (|| -> Result<(), String> {
        conn.execute("ATTACH DATABASE ?1 AS imported_package", params![package_db.to_string_lossy().as_ref()])
            .map_err(|e| format!("无法连接导出包数据库：{e}"))?;
        conn.execute_batch("CREATE TEMP TABLE importing_thread_ids (id TEXT PRIMARY KEY); CREATE TEMP TABLE importing_project_ids (id TEXT PRIMARY KEY)")
            .map_err(|e| format!("无法准备导入会话：{e}"))?;
        for id in &manifest.thread_ids {
            conn.execute("INSERT INTO importing_thread_ids (id) VALUES (?1)", params![id])
                .map_err(|e| format!("无法准备导入会话：{e}"))?;
        }
        for project_id in project_by_thread.values().flatten() {
            conn.execute("INSERT OR IGNORE INTO importing_project_ids (id) VALUES (?1)", params![project_id])
                .map_err(|e| format!("无法准备导入项目：{e}"))?;
        }
        let transaction = conn.transaction().map_err(|e| format!("无法开始导入：{e}"))?;
        transaction.execute("INSERT OR IGNORE INTO projects SELECT * FROM imported_package.projects WHERE id IN (SELECT id FROM importing_project_ids)", [])
            .map_err(|e| format!("无法导入项目配置：{e}"))?;
        transaction.execute("INSERT OR IGNORE INTO project_roots SELECT * FROM imported_package.project_roots WHERE project_id IN (SELECT id FROM importing_project_ids)", [])
            .map_err(|e| format!("无法导入项目目录配置：{e}"))?;
        transaction.execute("INSERT OR IGNORE INTO thread_sections SELECT * FROM imported_package.thread_sections WHERE id IN (SELECT DISTINCT thread_section_id FROM imported_package.threads WHERE id IN (SELECT id FROM importing_thread_ids) AND thread_section_id IS NOT NULL)", [])
            .map_err(|e| format!("无法导入会话分组配置：{e}"))?;
        transaction.execute("INSERT INTO threads SELECT * FROM imported_package.threads WHERE id IN (SELECT id FROM importing_thread_ids)", [])
            .map_err(|e| format!("无法导入会话记录：{e}"))?;
        transaction.execute("INSERT INTO thread_dynamic_tools SELECT * FROM imported_package.thread_dynamic_tools WHERE thread_id IN (SELECT id FROM importing_thread_ids)", [])
            .map_err(|e| format!("无法导入会话工具配置：{e}"))?;
        transaction.execute("INSERT INTO thread_artifacts SELECT * FROM imported_package.thread_artifacts WHERE thread_id IN (SELECT id FROM importing_thread_ids)", [])
            .map_err(|e| format!("无法导入会话产物配置：{e}"))?;
        transaction.execute("INSERT OR IGNORE INTO thread_spawn_edges SELECT * FROM imported_package.thread_spawn_edges WHERE child_thread_id IN (SELECT id FROM importing_thread_ids)", [])
            .map_err(|e| format!("无法导入子代理关系：{e}"))?;
        for (thread_id, rollout_path) in &rollout_paths {
            transaction.execute("UPDATE threads SET rollout_path = ?1 WHERE id = ?2", params![rollout_path, thread_id])
                .map_err(|e| format!("无法更新导入会话日志位置：{e}"))?;
        }
        for (thread_id, project_id) in &project_by_thread {
            if let Some(project_id) = project_id {
                transaction.execute("UPDATE threads SET project_id = ?1 WHERE id = ?2", params![project_id, thread_id])
                    .map_err(|e| format!("无法更新导入会话项目归属：{e}"))?;
            }
        }
        transaction.commit().map_err(|e| format!("无法提交导入：{e}"))?;
        conn.execute_batch("DETACH DATABASE imported_package").ok();
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&imported_logs_root);
        return Err(format!("{error}（导入前备份位于 {}）", safety.display()));
    }
    let assignment_count = update_global_state_assignments(&home, &project_by_thread)
        .map_err(|e| format!("会话已导入，但无法更新 Codex 侧栏归属：{e}（导入前备份位于 {}）", safety.display()))?;
    Ok(ActionResult {
        message: "已导入会话及项目配置。请重新打开 Codex，导入的会话会显示在对应项目中；原工作目录已保留，可继续使用路径修复。".into(),
        backup_folder: display_path(&safety),
        changes: vec![
            format!("导入会话记录：{} 个", manifest.thread_ids.len()),
            format!("导入项目配置：{} 个", manifest.projects.len()),
            format!("导入对话日志：{copied_logs} 个"),
            format!("更新侧栏项目归属：{assignment_count} 条"),
            format!("导入前备份文件：{} 个", backup_files.len()),
        ],
    })
}

#[tauri::command]
fn prepare_backup_directory(backup_base: Option<String>) -> Result<String, String> {
    let home = codex_home()?;
    Ok(display_path(&crate::backup_base(&home, backup_base.as_deref())?))
}

#[tauri::command]
fn list_repair_history(backup_base: Option<String>) -> Result<Vec<RepairHistoryItem>, String> {
    let home = codex_home()?;
    repair_history(&home, backup_base.as_deref())
}

#[tauri::command]
fn list_delete_history(backup_base: Option<String>) -> Result<Vec<DeleteHistoryItem>, String> {
    let home = codex_home()?;
    delete_history(&home, backup_base.as_deref())
}

#[tauri::command]
fn list_backup_history(backup_base: Option<String>) -> Result<Vec<BackupHistoryItem>, String> {
    let home = codex_home()?;
    backup_history(&home, backup_base.as_deref())
}

#[tauri::command]
fn cleanup_deleted_sidebar_references(backup_base: Option<String>) -> Result<usize, String> {
    let home = codex_home()?;
    let ids = delete_history(&home, backup_base.as_deref())?.into_iter()
        .filter(|item| item.deletion_kind == "session")
        .filter_map(|item| item.thread_id)
        .collect::<HashSet<_>>();
    let global_removed = purge_global_state_thread_references(&home, &ids)?;
    let catalog_removed = purge_desktop_catalog_thread_references(&home, &ids)?;
    Ok(global_removed + catalog_removed)
}

#[tauri::command]
fn delete_backup(request: BackupDeleteRequest) -> Result<(), String> {
    if request.confirmation != "DELETE_BACKUP" { return Err("删除备份确认无效。".into()); }
    let home = codex_home()?;
    let base = backup_base(&home, request.backup_base.as_deref())?;
    let folder = validate_backup_folder(&base, Path::new(&request.backup_folder))?;
    fs::remove_dir_all(&folder).map_err(|e| format!("删除备份失败：{e}"))
}

#[tauri::command]
fn clear_history(request: ClearHistoryRequest) -> Result<usize, String> {
    if request.confirmation != "CLEAR_HISTORY" { return Err("清除历史确认无效。".into()); }
    if !matches!(request.kind.as_str(), "repair" | "delete" | "backup") { return Err("未知的历史类型。".into()); }
    let home = codex_home()?;
    let base = backup_base(&home, request.backup_base.as_deref())?;
    let mut folders = Vec::new();
    for entry in fs::read_dir(&base).map_err(|e| format!("无法读取备份目录：{e}"))?.flatten() {
        let folder = entry.path();
        if !folder.is_dir() { continue; }
        let matches_kind = match request.kind.as_str() {
            "repair" => folder.join("repair-history.json").is_file(),
            "delete" => folder.join("delete-history.json").is_file(),
            "backup" => true,
            _ => false,
        };
        if matches_kind { folders.push(validate_backup_folder(&base, &folder)?); }
    }
    let count = folders.len();
    for folder in folders { fs::remove_dir_all(&folder).map_err(|e| format!("删除备份失败：{e}"))?; }
    Ok(count)
}

#[tauri::command]
fn close_codex_processes() -> Result<ProcessCloseResult, String> {
    let running = running_codex_processes()?;
    if running.is_empty() {
        return Ok(ProcessCloseResult { message: "没有检测到正在运行的 Codex 进程。".into(), requested: 0 });
    }
    let mut requested = 0;
    for (pid, _) in &running {
        if request_process_close(pid) {
            requested += 1;
        }
    }
    Ok(ProcessCloseResult {
        message: format!("已向 {requested} 个 Codex 进程发送关闭请求。等待它们完全退出后，再重新执行当前操作。"),
        requested,
    })
}

#[tauri::command]
fn repair_session(request: RepairRequest) -> Result<ActionResult, String> {
    if request.confirmation != "REPAIR" { return Err("请在确认框输入 REPAIR。".into()); }
    let target = PathBuf::from(request.target_path.trim());
    if !target.is_absolute() || !target.is_dir() { return Err("修复目标必须是一个已存在的绝对目录。".into()); }
    ensure_codex_is_closed()?;
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let mut conn = Connection::open(&db_path).map_err(|e| format!("无法打开状态数据库：{e}"))?;
    let mut logs_by_id: HashMap<String, Vec<LogMeta>> = HashMap::new();
    for log in scan_logs(&home) {
        logs_by_id.entry(log.id.clone()).or_default().push(log);
    }
    let targets = load_repair_targets(&conn, &request.thread_id, request.include_child_agents, &logs_by_id)?;
    let root = targets.first().ok_or("未找到目标会话。")?;
    let backup = backup_folder(&home, "repair", request.backup_base.as_deref())?;
    let mut backup_files = Vec::new();
    if let Some(entry) = backup_entry(&db_path, &backup, "state_5.sqlite")? { backup_files.push(entry); }
    for suffix in ["-wal", "-shm"] {
        let auxiliary = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        if let Some(name) = auxiliary.file_name().and_then(|part| part.to_str()) {
            if let Some(entry) = backup_entry(&auxiliary, &backup, name)? { backup_files.push(entry); }
        }
    }
    let mut log_index = 0;
    for target in &targets {
        for log in &target.logs {
            let name = format!("{log_index}-{}", log.path.file_name().unwrap_or_default().to_string_lossy());
            if let Some(entry) = backup_entry(&log.path, &backup, &name)? { backup_files.push(entry); }
            log_index += 1;
        }
    }
    let target_text = display_path(&target);
    let mut replacements = 0;
    let mut changes = Vec::new();
    let mut thread_changes = Vec::new();
    for repair_target in &targets {
        let mut old_paths = HashSet::from([repair_target.cwd.clone()]);
        for log in &repair_target.logs {
            if let Some(cwd) = &log.cwd { old_paths.insert(cwd.clone()); }
        }
        let label = if repair_target.id == request.thread_id { "主会话" } else { "子代理" };
        changes.push(format!("{label}数据库工作目录（{}）：{} → {}", repair_target.title, repair_target.cwd, target_text));
        for log in &repair_target.logs {
            let replaced = rewrite_log(&log.path, &old_paths, &target_text)?;
            replacements += replaced;
            let name = log.path.file_name().unwrap_or_default().to_string_lossy();
            if let Some(cwd) = &log.cwd {
                changes.push(format!("{label}对话工作目录（{name}）：{cwd} → {target_text}"));
            }
        }
        thread_changes.push(ThreadRepairChange {
            thread_id: repair_target.id.clone(),
            session_title: repair_target.title.clone(),
            source_cwd: repair_target.cwd.clone(),
            target_cwd: target_text.clone(),
        });
    }
    let transaction = conn.transaction().map_err(|e| format!("无法开始会话路径更新：{e}"))?;
    for repair_target in &targets {
        transaction.execute("UPDATE threads SET cwd = ?1 WHERE id = ?2", params![target_text, repair_target.id])
            .map_err(|e| format!("更新 Codex 会话路径失败（备份位于 {}）：{e}", backup.display()))?;
    }
    transaction.commit().map_err(|e| format!("无法提交会话路径更新：{e}"))?;
    let manifest = RepairManifest {
        version: 2,
        created_at: Local::now().to_rfc3339(),
        thread_id: request.thread_id,
        session_title: root.title.clone(),
        source_cwd: root.cwd.clone(),
        target_cwd: target_text,
        files: backup_files,
        thread_changes,
        rolled_back_at: None,
    };
    write_manifest(&backup, &manifest)?;
    let child_count = targets.len().saturating_sub(1);
    let scope = if child_count == 0 { "主会话".to_string() } else { format!("主会话及 {child_count} 个子代理") };
    Ok(ActionResult { message: format!("已修复{scope}的路径，并更新日志中的 {replacements} 处工作目录字段。请重启 Codex 后复查。"), backup_folder: display_path(&backup), changes })
}

#[tauri::command]
fn rollback_repair(request: RollbackRequest) -> Result<ActionResult, String> {
    if request.confirmation != "ROLLBACK" { return Err("回退确认无效。".into()); }
    ensure_codex_is_closed()?;
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let base = backup_base(&home, request.backup_base.as_deref())?;
    let manifest_path = validate_manifest_path(&base, Path::new(&request.manifest_path))?;
    let mut manifest = read_manifest(&manifest_path)?;
    if !(1..=2).contains(&manifest.version) || manifest.files.is_empty() { return Err("该修复历史不支持回退。".into()); }
    let folder = manifest_path.parent().ok_or("修复历史目录无效")?;
    for entry in &manifest.files {
        let original = PathBuf::from(&entry.original_path);
        let source = folder.join(&entry.backup_name);
        if !is_restorable_path(&home, &db_path, &original) || !source.is_file() {
            return Err(format!("修复历史包含无效或缺失的备份文件：{}", entry.backup_name));
        }
    }

    let thread_changes = if manifest.thread_changes.is_empty() {
        vec![ThreadRepairChange {
            thread_id: manifest.thread_id.clone(),
            session_title: manifest.session_title.clone(),
            source_cwd: manifest.source_cwd.clone(),
            target_cwd: manifest.target_cwd.clone(),
        }]
    } else {
        manifest.thread_changes.clone()
    };
    let change_by_id: HashMap<String, ThreadRepairChange> = thread_changes
        .iter().cloned().map(|change| (change.thread_id.clone(), change)).collect();
    let current_logs = scan_logs(&home).into_iter()
        .filter(|log| change_by_id.contains_key(&log.id))
        .collect::<Vec<_>>();
    let found_ids = current_logs.iter().map(|log| log.id.clone()).collect::<HashSet<_>>();
    if let Some(missing) = thread_changes.iter().find(|change| !found_ids.contains(&change.thread_id)) {
        return Err(format!("找不到会话“{}”当前的 JSONL 日志，已拒绝回退。", missing.session_title));
    }

    let safety = backup_folder(&home, "before-rollback", request.backup_base.as_deref())?;
    backup_database(&db_path, &safety)?;
    for (index, log) in current_logs.iter().enumerate() {
        let name = format!("current-{index}-{}", log.path.file_name().unwrap_or_default().to_string_lossy());
        backup_file(&log.path, &safety, &name)?;
    }

    let mut conn = Connection::open(&db_path).map_err(|e| format!("无法打开状态数据库：{e}"))?;
    let transaction = conn.transaction().map_err(|e| format!("无法开始回退：{e}"))?;
    for change in &thread_changes {
        let updated = transaction.execute(
            "UPDATE threads SET cwd = ?1 WHERE id = ?2",
            params![&change.source_cwd, &change.thread_id],
        ).map_err(|e| format!("回退数据库工作目录失败（回退前备份位于 {}）：{e}", safety.display()))?;
        if updated != 1 { return Err(format!("数据库中已找不到会话“{}”，已停止回退。", change.session_title)); }
    }
    transaction.commit().map_err(|e| format!("无法提交回退：{e}"))?;

    let mut replacements = 0;
    for log in &current_logs {
        let change = change_by_id.get(&log.id).ok_or("回退会话信息不完整。")?;
        replacements += rewrite_log(&log.path, &HashSet::from([change.target_cwd.clone()]), &change.source_cwd)?;
    }
    manifest.rolled_back_at = Some(Local::now().to_rfc3339());
    write_manifest(folder, &manifest)?;
    Ok(ActionResult {
        message: "已回退到本次修复前的状态。请重新启动 Codex 后复查。".into(),
        backup_folder: display_path(&safety),
        changes: vec![
            format!("数据库工作目录：{} → {}", manifest.target_cwd, manifest.source_cwd),
            format!("对话日志工作目录字段：恢复 {replacements} 处"),
            format!("回退前的当前状态已备份到：{}", safety.display()),
        ],
    })
}

#[tauri::command]
fn rollback_delete(request: DeleteRollbackRequest) -> Result<ActionResult, String> {
    if request.confirmation != "RESTORE_DELETION" { return Err("删除回退确认无效。".into()); }
    ensure_codex_is_closed()?;
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let base = backup_base(&home, request.backup_base.as_deref())?;
    let manifest_path = validate_delete_manifest_path(&base, Path::new(&request.manifest_path))?;
    let mut manifest = read_delete_manifest(&manifest_path)?;
    if manifest.completed_at.is_none() { return Err("该删除操作未完成，不能回退。".into()); }
    let folder = manifest_path.parent().ok_or("删除历史目录无效")?;
    let safety = backup_folder(&home, "before-delete-rollback", request.backup_base.as_deref())?;
    backup_database(&db_path, &safety)?;

    if manifest.deletion_kind == "orphan" {
        let entry = manifest.files.iter().find(|entry| entry.backup_name != "state_5.sqlite")
            .ok_or("删除备份中没有可恢复的日志文件。")?;
        let original = PathBuf::from(&entry.original_path);
        let source = folder.join(&entry.backup_name);
        if !is_managed_log(&home, &original) || !source.is_file() { return Err("删除备份中的日志文件无效或缺失。".into()); }
        if original.exists() { backup_file(&original, &safety, &format!("current-{}", entry.backup_name))?; }
        fs::create_dir_all(original.parent().ok_or("日志路径无效")?).map_err(|e| format!("无法创建日志目录：{e}"))?;
        fs::copy(&source, &original).map_err(|e| format!("恢复日志失败：{e}"))?;
        manifest.rolled_back_at = Some(Local::now().to_rfc3339());
        write_delete_manifest(folder, &manifest)?;
        return Ok(ActionResult { message: "已恢复删除的遗留日志。请重新扫描确认。".into(), backup_folder: display_path(&safety), changes: vec![format!("恢复日志：{}", original.display())] });
    }

    let thread_id = manifest.thread_id.clone().ok_or("删除历史缺少会话 ID。")?;
    let snapshot = folder.join("state_5.sqlite");
    if !snapshot.is_file() { return Err("删除备份中缺少状态数据库，不能回退。".into()); }
    let mut conn = Connection::open(&db_path).map_err(|e| format!("无法打开当前状态数据库：{e}"))?;
    conn.execute("ATTACH DATABASE ?1 AS deleted_backup", params![snapshot.to_string_lossy().as_ref()])
        .map_err(|e| format!("无法打开删除前的状态数据库备份：{e}"))?;
    conn.execute_batch("CREATE TEMP TABLE restored_thread_ids (id TEXT PRIMARY KEY)")
        .map_err(|e| format!("无法准备恢复会话：{e}"))?;
    let ids = {
        let mut statement = conn.prepare(
            "WITH RECURSIVE restored(id) AS (SELECT ?1 UNION SELECT edge.child_thread_id FROM deleted_backup.thread_spawn_edges edge JOIN restored ON edge.parent_thread_id = restored.id) SELECT id FROM restored"
        ).map_err(|e| format!("无法读取删除备份：{e}"))?;
        let ids = statement.query_map(params![&thread_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("无法读取删除会话范围：{e}"))?
            .collect::<Result<Vec<_>, _>>().map_err(|e| format!("无法读取删除会话范围：{e}"))?;
        ids
    };
    if ids.is_empty() { return Err("删除备份中找不到该会话。".into()); }
    for id in &ids {
        let exists: Option<String> = conn.query_row("SELECT id FROM threads WHERE id = ?1", params![id], |row| row.get(0)).optional()
            .map_err(|e| format!("无法检查当前会话：{e}"))?;
        if exists.is_some() { return Err("当前数据库已存在同 ID 会话，已拒绝覆盖。".into()); }
        conn.execute("INSERT INTO restored_thread_ids (id) VALUES (?1)", params![id])
            .map_err(|e| format!("无法准备恢复会话：{e}"))?;
    }
    let log_entries = manifest.files.iter().filter(|entry| is_managed_log(&home, Path::new(&entry.original_path))).cloned().collect::<Vec<_>>();
    for entry in &log_entries {
        let source = folder.join(&entry.backup_name);
        if !source.is_file() { return Err(format!("删除备份缺少日志：{}", entry.backup_name)); }
        let original = PathBuf::from(&entry.original_path);
        if original.exists() { backup_file(&original, &safety, &format!("current-{}", entry.backup_name))?; }
    }
    {
        let transaction = conn.transaction().map_err(|e| format!("无法开始删除回退：{e}"))?;
        transaction.execute("INSERT INTO threads SELECT * FROM deleted_backup.threads WHERE id IN (SELECT id FROM restored_thread_ids)", [])
            .map_err(|e| format!("恢复会话记录失败：{e}"))?;
        transaction.execute("INSERT INTO thread_dynamic_tools SELECT * FROM deleted_backup.thread_dynamic_tools WHERE thread_id IN (SELECT id FROM restored_thread_ids)", []).map_err(|e| format!("恢复会话工具失败：{e}"))?;
        transaction.execute("INSERT INTO thread_artifacts SELECT * FROM deleted_backup.thread_artifacts WHERE thread_id IN (SELECT id FROM restored_thread_ids)", []).map_err(|e| format!("恢复会话产物失败：{e}"))?;
        transaction.execute("INSERT OR IGNORE INTO thread_spawn_edges SELECT * FROM deleted_backup.thread_spawn_edges WHERE child_thread_id IN (SELECT id FROM restored_thread_ids)", []).map_err(|e| format!("恢复会话关系失败：{e}"))?;
        transaction.commit().map_err(|e| format!("无法提交删除回退：{e}"))?;
    }
    for entry in &log_entries {
        let original = PathBuf::from(&entry.original_path);
        fs::create_dir_all(original.parent().ok_or("日志路径无效")?).map_err(|e| format!("无法创建日志目录：{e}"))?;
        fs::copy(folder.join(&entry.backup_name), &original).map_err(|e| format!("恢复会话日志失败：{e}"))?;
    }
    conn.execute_batch("DETACH DATABASE deleted_backup").ok();
    manifest.rolled_back_at = Some(Local::now().to_rfc3339());
    write_delete_manifest(folder, &manifest)?;
    Ok(ActionResult { message: format!("已恢复会话“{}”及其 {} 个子代理。请重新打开 Codex 后确认。", manifest.session_title, ids.len().saturating_sub(1)), backup_folder: display_path(&safety), changes: vec![format!("恢复会话记录：{} 个", ids.len()), format!("恢复对话日志：{} 个", log_entries.len())] })
}

#[tauri::command]
fn delete_session(request: DeleteSessionRequest) -> Result<ActionResult, String> {
    if request.confirmation != "DELETE" { return Err("请在确认框输入 DELETE。".into()); }
    // Codex Desktop keeps its sidebar in memory.  Deleting through a separate
    // app-server process while it is open leaves a stale row that cannot be
    // restored, even though the database deletion succeeded.
    ensure_codex_is_closed()?;
    let home = codex_home()?;
    let db_path = state_database(&home)?;
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("无法打开 Codex 状态数据库：{e}"))?;
    let (title, source): (String, String) = conn.query_row(
        "SELECT COALESCE(NULLIF(TRIM(name), ''), title), source FROM threads WHERE id = ?1",
        params![&request.thread_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|e| format!("找不到要删除的会话，扫描结果可能已过期：{e}"))?;
    if source != "vscode" { return Err("只能删除 Codex 侧边栏中的普通会话。".into()); }

    let mut thread_ids = HashSet::from([request.thread_id.clone()]);
    thread_ids.extend(descendant_thread_ids(&conn, &request.thread_id)?);
    drop(conn);

    let backup = backup_folder(&home, "deleted-session", request.backup_base.as_deref())?;
    backup_database(&db_path, &backup)?;
    let mut backup_files = vec![BackupFileEntry {
        original_path: display_path(&db_path),
        backup_name: "state_5.sqlite".into(),
    }];
    if let Some(entry) = backup_entry(&home.join(".codex-global-state.json"), &backup, "codex-global-state.json")? {
        backup_files.push(entry);
    }
    let catalog_db = desktop_catalog_database(&home);
    if let Some(entry) = backup_entry(&catalog_db, &backup, "codex-dev.db")? {
        backup_files.push(entry);
    }
    let logs = scan_logs(&home).into_iter()
        .filter(|log| thread_ids.contains(&log.id))
        .collect::<Vec<_>>();
    for (index, log) in logs.iter().enumerate() {
        let name = format!("{index}-{}", log.path.file_name().unwrap_or_default().to_string_lossy());
        if let Some(entry) = backup_entry(&log.path, &backup, &name)? { backup_files.push(entry); }
    }

    let child_count = thread_ids.len().saturating_sub(1);
    let mut manifest = DeleteManifest {
        version: 1,
        created_at: Local::now().to_rfc3339(),
        completed_at: None,
        deletion_kind: "session".into(),
        thread_id: Some(request.thread_id.clone()),
        session_title: title.clone(),
        source_path: None,
        child_count,
        files: backup_files,
        rolled_back_at: None,
    };
    write_delete_manifest(&backup, &manifest)?;

    delete_thread_with_codex(&home, &request.thread_id)
        .map_err(|error| format!("删除会话失败；操作前备份位于 {}：{error}", backup.display()))?;
    let sidebar_references = purge_global_state_thread_references(&home, &thread_ids)
        .map_err(|error| format!("会话已从状态库删除，但无法清理 Codex 侧栏状态；操作前备份位于 {}：{error}", backup.display()))?;
    let catalog_references = purge_desktop_catalog_thread_references(&home, &thread_ids)
        .map_err(|error| format!("会话已从状态库删除，但无法清理 Codex 桌面目录缓存；操作前备份位于 {}：{error}", backup.display()))?;
    manifest.completed_at = Some(Local::now().to_rfc3339());
    write_delete_manifest(&backup, &manifest)?;
    let child_note = if child_count == 0 { String::new() } else { format!("及其 {child_count} 个子代理") };
    Ok(ActionResult {
        message: format!("已通过 Codex 删除会话“{title}”{child_note}，并清理 {sidebar_references} 条侧栏状态引用和 {catalog_references} 条桌面目录缓存。重新打开 Codex 后即可看到结果。"),
        backup_folder: display_path(&backup),
        changes: vec![format!("删除的会话：{}（{}）", title, request.thread_id)],
    })
}

#[tauri::command]
fn delete_orphan(request: DeleteRequest) -> Result<ActionResult, String> {
    if request.confirmation != "DELETE" { return Err("请在确认框输入 DELETE。".into()); }
    ensure_codex_is_closed()?;
    let home = codex_home()?;
    let target = PathBuf::from(&request.log_path);
    if !target.is_file() || !is_managed_log(&home, &target) { return Err("目标不是 Codex 会话目录中的 JSONL 文件。".into()); }
    let current = report()?;
    if !current.orphans.iter().any(|record| record.log_path == request.log_path) {
        return Err("此记录仍被 Codex 当前数据库引用，或扫描结果已过期；请重新扫描。".into());
    }
    let backup = backup_folder(&home, "deleted-orphan", request.backup_base.as_deref())?;
    let backup_name = target.file_name().and_then(|item| item.to_str()).unwrap_or("orphan.jsonl");
    let backup_entry = backup_entry(&target, &backup, backup_name)?.ok_or("无法备份要删除的遗留日志。")?;
    let mut manifest = DeleteManifest {
        version: 1,
        created_at: Local::now().to_rfc3339(),
        completed_at: None,
        deletion_kind: "orphan".into(),
        thread_id: None,
        session_title: target.file_name().and_then(|item| item.to_str()).unwrap_or("遗留日志").into(),
        source_path: Some(display_path(&target)),
        child_count: 0,
        files: vec![backup_entry],
        rolled_back_at: None,
    };
    write_delete_manifest(&backup, &manifest)?;
    fs::remove_file(&target).map_err(|e| format!("删除遗留日志失败：{e}"))?;
    manifest.completed_at = Some(Local::now().to_rfc3339());
    write_delete_manifest(&backup, &manifest)?;
    Ok(ActionResult { message: "已从 Codex 会话目录删除该未引用日志；原文件已备份。".into(), backup_folder: display_path(&backup), changes: vec![format!("删除的遗留日志：{}", target.display())] })
}

fn update_zoom<R: tauri::Runtime>(app: &tauri::AppHandle<R>, requested: f64) {
    let zoom = requested.clamp(MIN_ZOOM, MAX_ZOOM);
    if let Ok(mut current) = app.state::<ZoomLevel>().0.lock() {
        *current = zoom;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_zoom(zoom);
    }
}

fn current_zoom<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> f64 {
    app.state::<ZoomLevel>()
        .0
        .lock()
        .map(|zoom| *zoom)
        .unwrap_or(DEFAULT_ZOOM)
}

fn set_predefined_menu_text<R: tauri::Runtime>(
    submenu: &tauri::menu::Submenu<R>,
    position: usize,
    text: &str,
) -> tauri::Result<()> {
    if let Some(MenuItemKind::Predefined(item)) = submenu.items()?.get(position) {
        item.set_text(text)?;
    }
    Ok(())
}

fn application_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;

    #[cfg(target_os = "macos")]
    if let Some(MenuItemKind::Submenu(application_submenu)) = menu.items()?.into_iter().next() {
        // Replace the generic About item so the native panel receives the
        // product identity explicitly, including a Dock-sized icon and credits.
        application_submenu.remove_at(0)?;
        let metadata = AboutMetadata {
            name: Some("Codex Session Manager".into()),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            copyright: Some("Copyright © 2026 H-Knight".into()),
            credits: Some("作者：H-Knight".into()),
            icon: Some(tauri::image::Image::from_bytes(include_bytes!(
                "../icons/icon-macos.png"
            ))?),
            ..Default::default()
        };
        let about = PredefinedMenuItem::about(
            app,
            Some("关于 Codex Session Manager"),
            Some(metadata),
        )?;
        application_submenu.prepend(&about)?;
        set_predefined_menu_text(&application_submenu, 2, "服务")?;
        set_predefined_menu_text(&application_submenu, 4, "隐藏 Codex Session Manager")?;
        set_predefined_menu_text(&application_submenu, 5, "隐藏其他应用")?;
        set_predefined_menu_text(&application_submenu, 7, "退出 Codex Session Manager")?;
    }

    let zoom_in = MenuItemBuilder::with_id("view.zoom-in", "放大")
        .accelerator("CmdOrCtrl+Equal")
        .build(app)?;
    let zoom_out = MenuItemBuilder::with_id("view.zoom-out", "缩小")
        .accelerator("CmdOrCtrl+Minus")
        .build(app)?;
    let reset_zoom = MenuItemBuilder::with_id("view.zoom-reset", "实际大小")
        .accelerator("CmdOrCtrl+0")
        .build(app)?;

    let mut view_submenu = None;
    for item in menu.items()? {
        let MenuItemKind::Submenu(submenu) = item else { continue };
        match submenu.text()?.as_str() {
            "File" => {
                submenu.set_text("文件")?;
                set_predefined_menu_text(&submenu, 0, "关闭窗口")?;
                #[cfg(not(target_os = "macos"))]
                set_predefined_menu_text(&submenu, 1, "退出 Codex Session Manager")?;
            }
            "Edit" => {
                submenu.set_text("编辑")?;
                set_predefined_menu_text(&submenu, 0, "撤销")?;
                set_predefined_menu_text(&submenu, 1, "重做")?;
                set_predefined_menu_text(&submenu, 3, "剪切")?;
                set_predefined_menu_text(&submenu, 4, "复制")?;
                set_predefined_menu_text(&submenu, 5, "粘贴")?;
                set_predefined_menu_text(&submenu, 6, "全选")?;
            }
            "View" => {
                submenu.set_text("显示")?;
                set_predefined_menu_text(&submenu, 0, "进入全屏幕")?;
                view_submenu = Some(submenu);
            }
            "Window" => {
                submenu.set_text("窗口")?;
                set_predefined_menu_text(&submenu, 0, "最小化")?;
                set_predefined_menu_text(&submenu, 1, "缩放")?;
                let close_position = submenu.items()?.len().saturating_sub(1);
                set_predefined_menu_text(&submenu, close_position, "关闭窗口")?;
            }
            "Help" => {
                submenu.set_text("帮助")?;
                #[cfg(not(target_os = "macos"))]
                set_predefined_menu_text(&submenu, 0, "关于 Codex Session Manager")?;
            }
            _ => {}
        }
    }

    let zoom_items: [&dyn tauri::menu::IsMenuItem<R>; 3] = [&zoom_in, &zoom_out, &reset_zoom];
    if let Some(view) = view_submenu {
        view.insert_items(&zoom_items, 0)?;
    } else {
        let view = SubmenuBuilder::new(app, "显示").items(&zoom_items).build()?;
        menu.append(&view)?;
    }
    Ok(menu)
}

pub fn run() {
    tauri::Builder::default()
        .manage(ZoomLevel(Mutex::new(DEFAULT_ZOOM)))
        .menu(application_menu)
        .on_menu_event(|app, event| {
            if event.id() == "view.zoom-in" {
                update_zoom(app, current_zoom(app) + ZOOM_STEP);
            } else if event.id() == "view.zoom-out" {
                update_zoom(app, current_zoom(app) - ZOOM_STEP);
            } else if event.id() == "view.zoom-reset" {
                update_zoom(app, DEFAULT_ZOOM);
            }
        })
        // Register first so a second launch exits before it can create another window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![scan_codex, inspect_export_package, export_sessions, import_sessions, prepare_backup_directory, list_repair_history, list_delete_history, list_backup_history, cleanup_deleted_sidebar_references, delete_backup, clear_history, close_codex_processes, repair_session, rollback_repair, rollback_delete, delete_session, delete_orphan])
        .run(tauri::generate_context!())
        .expect("error while running Codex Session Manager");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a populated local Codex database; run explicitly with --ignored"]
    fn audit_counts_only_sidebar_visible_sessions() {
        let home = codex_home().expect("Codex home should exist for the local integration test");
        let database = state_database(&home).expect("Codex state database should exist");
        let conn = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("state database should be readable");
        let projects = query_projects(&conn).expect("projects should be queryable");
        let assignments = load_desktop_project_assignments(&home);
        let visible_threads = query_threads(&conn).expect("threads should be queryable")
            .into_iter().filter(|thread| thread.source == "vscode").collect::<Vec<_>>();
        let audit = report().expect("audit should scan the local Codex store");
        assert_eq!(audit.sessions.len(), visible_threads.len());

        for project in &audit.projects {
            let expected_for_root = visible_threads.iter()
                .filter(|thread| effective_project_id(thread, &projects, &assignments).as_deref() == Some(&project.id))
                .count();
            assert_eq!(project.session_count, expected_for_root, "{}", project.name);
        }
    }

    #[test]
    #[ignore = "requires local Codex data with a session assigned across a migrated root"]
    fn audit_keeps_sessions_with_their_project_after_a_root_change() {
        let home = codex_home().expect("Codex home should exist for the local integration test");
        let database = state_database(&home).expect("Codex state database should exist");
        let conn = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("state database should be readable");
        let projects = query_projects(&conn).expect("projects should be queryable");
        let assignments = load_desktop_project_assignments(&home);
        let threads = query_threads(&conn).expect("threads should be queryable");
        let audit = report().expect("audit should scan the local Codex store");

        let migrated = threads.iter().find(|thread| {
            thread.source == "vscode"
                && assignments.contains_key(&thread.id)
                && effective_project_id(thread, &projects, &assignments).is_some_and(|project_id| {
                    projects.iter().find(|project| project.id == project_id)
                        .is_some_and(|project| !project.roots.iter().any(|root| is_same_path(root, &thread.cwd)))
                })
        }).expect("the local data should contain a session assigned across a migrated root");

        let expected_project = effective_project_id(migrated, &projects, &assignments).unwrap();
        let audited = audit.sessions.iter().find(|session| session.id == migrated.id)
            .expect("migrated visible session should appear in the audit");
        assert_eq!(audited.project_id.as_deref(), Some(expected_project.as_str()));
        assert_eq!(audited.status, "mismatch");
    }

    #[test]
    #[ignore = "requires a populated local Codex database with a named sidebar session"]
    fn audit_uses_the_codex_sidebar_name_when_present() {
        let home = codex_home().expect("Codex home should exist for the local integration test");
        let database = state_database(&home).expect("Codex state database should exist");
        let conn = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("state database should be readable");
        let (id, sidebar_name): (String, String) = conn.query_row(
            "SELECT id, name FROM threads WHERE source = 'vscode' AND TRIM(COALESCE(name, '')) <> '' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).expect("the local Codex database should contain a named sidebar session");
        let audited_title = query_threads(&conn).expect("threads should be queryable")
            .into_iter().find(|thread| thread.id == id)
            .expect("the named sidebar session should be present").title;
        assert_eq!(audited_title, sidebar_name);
    }

    #[test]
    fn identifies_only_codex_processes_that_can_update_local_state() {
        assert!(is_codex_process("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"));
        assert!(is_codex_process("/Applications/ChatGPT.app/Contents/Resources/codex app-server"));
        assert!(is_codex_process("Codex.exe"));
        assert!(is_codex_process(r"C:\Program Files\OpenAI\ChatGPT.exe"));
        assert!(!is_codex_process("target/debug/codex-session-manager"));
        assert!(!is_codex_process("codex-session-manager.exe"));
        assert!(!is_codex_process("node /workspace/node_modules/.bin/vite"));
    }

    #[test]
    fn parses_only_matching_codex_processes() {
        let line = "  123 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT";
        let mut fields = line.trim().splitn(2, char::is_whitespace);
        assert_eq!(fields.next(), Some("123"));
        assert!(is_codex_process(fields.next().unwrap()));
    }

    #[test]
    fn parses_windows_tasklist_csv_without_localized_headers() {
        let process = parse_windows_tasklist_line(
            r#""Codex.exe","4242","Console","1","81,920 K""#,
        ).expect("tasklist row should parse");
        assert_eq!(process, ("4242".into(), "Codex.exe".into()));
        assert!(parse_windows_tasklist_line("INFO: No tasks are running").is_none());
    }

    #[test]
    fn windows_path_comparison_handles_separators_case_and_device_prefixes() {
        assert_eq!(
            normalized_for_platform(r"C:\Users\Knight\Project\", true),
            "c:/users/knight/project"
        );
        assert_eq!(
            normalized_for_platform(r"\\?\C:\Users\KNIGHT\Project", true),
            "c:/users/knight/project"
        );
        assert_eq!(normalized_for_platform(r"C:\", true), "c:/");
        assert_ne!(
            normalized_for_platform("/Users/Knight/Project", false),
            normalized_for_platform("/users/knight/project", false)
        );
    }

    #[test]
    fn atomic_write_replaces_an_existing_file() {
        let folder = std::env::temp_dir().join(format!(
            "codex-session-manager-atomic-write-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&folder).expect("temporary folder should be created");
        let path = folder.join("session.jsonl");
        fs::write(&path, "old\n").expect("old file should be written");
        atomic_write(&path, "new\n").expect("existing file should be replaced");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        fs::remove_dir_all(folder).expect("temporary folder should be removed");
    }

    #[test]
    fn purges_only_the_deleted_thread_references_from_global_state() {
        let mut state = serde_json::json!({
            "thread-project-assignments": { "deleted": { "projectId": "project-a" }, "kept": { "projectId": "project-b" } },
            "pinned-thread-ids": ["deleted", "kept"],
            "new-thread:client-a": "deleted",
            "client-new-thread:client-b": "deleted",
            "thread-reference-capability:deleted": true,
            "unrelated": "deleted"
        });
        assert_eq!(purge_thread_reference(&mut state, "deleted"), 5);
        assert!(state["thread-project-assignments"].get("deleted").is_none());
        assert_eq!(state["thread-project-assignments"]["kept"]["projectId"], "project-b");
        assert_eq!(state["pinned-thread-ids"], serde_json::json!(["kept"]));
        assert!(state.get("new-thread:client-a").is_none());
        assert!(state.get("client-new-thread:client-b").is_none());
        assert!(state.get("thread-reference-capability:deleted").is_none());
        assert_eq!(state["unrelated"], "deleted");
    }

    #[test]
    fn migration_updates_only_cwd_fields_and_keeps_file_links() {
        let old = "/Users/knight/Desktop/Test";
        let target = "/Users/knight/Desktop/Test2";
        let mut value = serde_json::json!({
            "cwd": old,
            "message": "[Test1](</Users/knight/Desktop/Test/Test1.md>)",
            "nested": { "cwd": old }
        });
        let count = replace_path_values(&mut value, &HashSet::from([old.to_string()]), target);
        let rewritten = value.to_string();
        assert_eq!(count, 2);
        assert!(rewritten.contains("\"cwd\":\"/Users/knight/Desktop/Test2\""));
        assert!(rewritten.contains("/Users/knight/Desktop/Test/Test1.md"));
    }

    #[test]
    fn repair_history_manifest_round_trips() {
        let folder = std::env::temp_dir().join(format!(
            "codex-session-manager-history-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&folder).expect("temporary history folder should be created");
        let manifest = RepairManifest {
            version: 2,
            created_at: "2026-09-05T00:00:00+08:00".into(),
            thread_id: "thread-test".into(),
            session_title: "测试会话".into(),
            source_cwd: "/old".into(),
            target_cwd: "/new".into(),
            files: vec![BackupFileEntry { original_path: "/original".into(), backup_name: "backup".into() }],
            thread_changes: vec![ThreadRepairChange {
                thread_id: "child-test".into(),
                session_title: "子代理".into(),
                source_cwd: "/old-child".into(),
                target_cwd: "/new".into(),
            }],
            rolled_back_at: None,
        };
        let path = write_manifest(&folder, &manifest).expect("manifest should be written");
        let restored = read_manifest(&path).expect("manifest should be readable");
        assert_eq!(restored.thread_id, manifest.thread_id);
        assert_eq!(restored.source_cwd, "/old");
        assert_eq!(restored.files.len(), 1);
        assert_eq!(restored.thread_changes.len(), 1);
        fs::remove_dir_all(folder).expect("temporary history folder should be removed");
    }

    #[test]
    fn delete_history_lists_only_completed_operations() {
        let base = std::env::temp_dir().join(format!(
            "codex-session-manager-delete-history-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let completed_folder = base.join("completed-deletion");
        let incomplete_folder = base.join("incomplete-deletion");
        fs::create_dir_all(&completed_folder).expect("completed deletion folder should be created");
        fs::create_dir_all(&incomplete_folder).expect("incomplete deletion folder should be created");
        let manifest = DeleteManifest {
            version: 1,
            created_at: "2026-09-05T00:00:00+08:00".into(),
            completed_at: Some("2026-09-05T00:00:01+08:00".into()),
            deletion_kind: "session".into(),
            thread_id: Some("thread-test".into()),
            session_title: "测试删除会话".into(),
            source_path: None,
            child_count: 2,
            files: vec![BackupFileEntry { original_path: "/original".into(), backup_name: "backup".into() }],
            rolled_back_at: None,
        };
        write_delete_manifest(&completed_folder, &manifest).expect("completed manifest should be written");
        let mut incomplete = manifest.clone();
        incomplete.completed_at = None;
        write_delete_manifest(&incomplete_folder, &incomplete).expect("incomplete manifest should be written");

        let items = delete_history(&base, Some(base.to_str().expect("temporary path should be UTF-8")))
            .expect("delete history should load");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].thread_id.as_deref(), Some("thread-test"));
        assert_eq!(items[0].child_count, 2);
        assert_eq!(items[0].file_count, 1);
        fs::remove_dir_all(base).expect("temporary history folder should be removed");
    }

    #[test]
    fn legacy_repair_manifest_remains_readable() {
        let legacy = r#"{
          "version": 1,
          "createdAt": "2026-09-05T00:00:00+08:00",
          "threadId": "thread-test",
          "sessionTitle": "测试会话",
          "sourceCwd": "/old",
          "targetCwd": "/new",
          "files": [],
          "rolledBackAt": null
        }"#;
        let manifest: RepairManifest = serde_json::from_str(legacy).expect("legacy manifest should deserialize");
        assert_eq!(manifest.version, 1);
        assert!(manifest.thread_changes.is_empty());
    }

    #[test]
    fn finds_all_descendant_agents_once() {
        let conn = Connection::open_in_memory().expect("in-memory database should open");
        conn.execute_batch(
            "CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT NOT NULL PRIMARY KEY,
                status TEXT NOT NULL
            );
            INSERT INTO thread_spawn_edges VALUES
                ('root', 'child-a', 'completed'),
                ('root', 'child-b', 'completed'),
                ('child-a', 'grandchild', 'completed');",
        ).expect("test edges should be created");
        assert_eq!(descendant_thread_ids(&conn, "root").expect("descendants should load"), vec!["child-a", "child-b", "grandchild"]);
    }

    #[test]
    fn parses_successful_thread_delete_response() {
        let output = r#"{"id":1,"result":{"userAgent":"Codex"}}
{"id":2,"result":{}}
{"method":"thread/deleted","params":{"threadId":"thread-test"}}"#;
        assert!(parse_thread_delete_response(output).is_ok());
    }

    #[test]
    fn returns_thread_delete_error_message() {
        let output = r#"{"error":{"code":-32603,"message":"thread is active"},"id":2}"#;
        assert_eq!(parse_thread_delete_response(output).unwrap_err(), "thread is active");
    }

    #[test]
    #[ignore = "requires a populated local Codex database; run explicitly with --ignored"]
    fn trimmed_export_snapshot_keeps_only_the_requested_thread() {
        let home = codex_home().expect("Codex home should exist for the local integration test");
        let database = state_database(&home).expect("state database should exist");
        let source = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("state database should be readable");
        let thread_id: String = source.query_row(
            "SELECT id FROM threads WHERE source = 'vscode' ORDER BY recency_at_ms DESC LIMIT 1",
            [],
            |row| row.get(0),
        ).expect("a visible Codex session should exist");
        let temporary = std::env::temp_dir().join(format!(
            "codex-session-manager-export-test-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temporary).expect("temporary export folder should be created");
        backup_database(&database, &temporary).expect("snapshot should be created");
        trim_export_database(&temporary.join("state_5.sqlite"), std::slice::from_ref(&thread_id), &HashSet::new())
            .expect("snapshot should be trimmed");
        let trimmed = Connection::open(temporary.join("state_5.sqlite")).expect("trimmed snapshot should open");
        let count: i64 = trimmed.query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .expect("trimmed thread count should load");
        let kept: String = trimmed.query_row("SELECT id FROM threads", [], |row| row.get(0))
            .expect("requested thread should remain");
        assert_eq!(count, 1);
        assert_eq!(kept, thread_id);
        fs::remove_dir_all(temporary).expect("temporary export folder should be removed");
    }
}
