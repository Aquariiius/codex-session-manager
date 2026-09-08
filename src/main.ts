import { invoke } from "@tauri-apps/api/core";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import "./style.css";

type Status = "match" | "mismatch" | "unlinked" | "missing_log";

interface Project {
  id: string;
  name: string;
  roots: string[];
  sessionCount: number;
  issueCount: number;
}

interface Session {
  id: string;
  title: string;
  projectId: string | null;
  projectName: string | null;
  projectRoots: string[];
  databaseCwd: string;
  conversationCwd: string | null;
  logPath: string | null;
  archived: boolean;
  status: Status;
}

interface OrphanRecord {
  id: string;
  conversationCwd: string | null;
  logPath: string;
  archived: boolean;
}

interface AuditReport {
  codexHome: string;
  stateDatabase: string;
  defaultBackupDirectory: string;
  projects: Project[];
  sessions: Session[];
  orphans: OrphanRecord[];
  summary: {
    projects: number;
    sessions: number;
    matched: number;
    mismatched: number;
    unlinked: number;
    missingLogs: number;
    orphanRecords: number;
  };
}

interface ActionResult {
  message: string;
  backupFolder: string;
  changes: string[];
}

interface ProcessCloseResult {
  message: string;
  requested: number;
}

interface Notice {
  title: string;
  message: string;
  tone: "warning" | "success";
  canCloseCodex?: boolean;
}

type PendingDeletion =
  | { kind: "session"; session: Session }
  | { kind: "orphan"; logPath: string };

interface RepairHistoryItem {
  createdAt: string;
  threadId: string;
  sessionTitle: string;
  sourceCwd: string;
  targetCwd: string;
  backupFolder: string;
  manifestPath: string;
  fileCount: number;
  rolledBackAt: string | null;
}

interface DeleteHistoryItem {
  createdAt: string;
  deletionKind: "session" | "orphan";
  threadId: string | null;
  sessionTitle: string;
  sourcePath: string | null;
  childCount: number;
  backupFolder: string;
  fileCount: number;
  rolledBackAt: string | null;
}

interface BackupHistoryItem {
  folder: string;
  name: string;
  operation: string;
  createdAt: string;
  fileCount: number;
  totalSize: number;
}

interface ExportProject {
  id: string;
  name: string;
  roots: string[];
}

interface ImportPackageInfo {
  manifestPath: string;
  exportedAt: string;
  projects: ExportProject[];
  visibleThreadCount: number;
  threadCount: number;
  logCount: number;
}

let report: AuditReport | null = null;
let repairHistory: RepairHistoryItem[] = [];
let deleteHistory: DeleteHistoryItem[] = [];
let backupHistory: BackupHistoryItem[] = [];
let activeTab: "sessions" | "orphans" | "history" | "delete-history" | "backup-history" = "sessions";
let selectedProject = "all";
let selectedSession: Session | null = null;
let healthFilter: "all" | "mismatch" | "unlinked" | "missing_log" | "healthy" = "all";
let theme: "light" | "dark" = localStorage.getItem("theme") === "light" ? "light" : "dark";
let backupBase = localStorage.getItem("backup-base");
let showBackupSettings = false;
let notice: Notice | null = null;
let pendingDeletion: PendingDeletion | null = null;
let showExportDialog = false;
let exportSelection = new Set<string>();
let exportDirectory = "";
let exportProject = "all";
let collapsedExportProjects = new Set<string>();
let importPackage: ImportPackageInfo | null = null;

const app = document.querySelector<HTMLDivElement>("#app")!;

function escapeHtml(value: string | null | undefined): string {
  return (value ?? "—")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function displayTitle(value: string): string {
  const normalized = value.replaceAll(/\s+/g, " ").trim();
  return normalized.length > 72 ? `${normalized.slice(0, 72)}…` : normalized || "未命名会话";
}

function statusLabel(status: Status): string {
  return { match: "路径一致", mismatch: "路径不一致", unlinked: "未归属项目", missing_log: "找不到日志" }[status];
}

function statusHint(session: Session): string {
  if (session.status === "match") return "项目根目录、Codex 数据库工作目录和会话日志工作目录相同。";
  if (session.status === "mismatch") {
    const projectMatchesDatabase = session.projectRoots.some((root) => samePath(root, session.databaseCwd));
    const databaseMatchesConversation = samePath(session.databaseCwd, session.conversationCwd);
    const issues = [];
    if (!projectMatchesDatabase) issues.push("项目目录与数据库工作目录不同");
    if (!databaseMatchesConversation) issues.push("数据库工作目录与对话工作目录不同");
    return issues.join("；") || "项目目录与会话工作目录不同。";
  }
  if (session.status === "unlinked") return "该会话没有关联到当前 Codex 项目，无法判定项目路径。";
  return "当前数据库会话没有能读取的 session_meta JSONL 日志。";
}

function samePath(left: string | null | undefined, right: string | null | undefined): boolean {
  if (!left || !right) return false;
  return left.replace(/[\\/]+$/, "") === right.replace(/[\\/]+$/, "");
}

function filterByHealth(sessions: Session[]): Session[] {
  if (healthFilter === "healthy") return sessions.filter((session) => session.status === "match");
  if (healthFilter === "mismatch" || healthFilter === "unlinked" || healthFilter === "missing_log") {
    return sessions.filter((session) => session.status === healthFilter);
  }
  return sessions;
}

function healthFilterButton(value: typeof healthFilter, label: string, count: number): string {
  return `<button class="health-filter health-${value} ${healthFilter === value ? "selected" : ""}" data-health="${value}">${label}<span>${count}</span></button>`;
}

function render(): void {
  document.documentElement.dataset.theme = theme;
  if (!report) {
    app.innerHTML = `<section class="empty"><div class="spinner"></div><h1>正在读取本地 Codex 数据</h1><p>扫描项目、当前会话和历史 JSONL 记录。</p></section>`;
    return;
  }
  const filteredByHealth = filterByHealth(report.sessions);
  const filtered = filteredByHealth
    .filter((session) => selectedProject === "all" || session.projectId === selectedProject);
  app.innerHTML = `
    <header class="topbar">
      <div><p class="eyebrow">LOCAL CODEX AUDIT</p><h1>Codex 会话管理</h1></div>
      <div class="topbar-actions"><button id="export-sessions" class="button secondary">导出会话</button><button id="import-sessions" class="button secondary">导入会话</button><button id="backup-settings" class="button secondary">备份目录</button><button id="theme-toggle" class="button secondary" aria-label="切换主题">${theme === "dark" ? "☀ 浅色主题" : "◐ 深色主题"}</button><button id="rescan" class="button secondary">重新扫描</button></div>
    </header>
    <section class="summary-grid">
      ${metric("项目", report.summary.projects, "neutral")}
      ${metric("会话", report.summary.sessions, "neutral")}
      ${metric("路径一致", report.summary.matched, "good")}
      ${metric("问题会话", report.summary.sessions - report.summary.matched, "bad")}
      ${metric("遗留记录", report.summary.orphanRecords, "warn")}
    </section>
    <section class="source-note">
      <span>当前数据源</span><code>${escapeHtml(report.stateDatabase)}</code>
      <span>会话日志</span><code>${escapeHtml(report.codexHome)}/sessions · archived_sessions</code>
    </section>
    <nav class="tabs" aria-label="页面">
      <button class="tab ${activeTab === "sessions" ? "active" : ""}" data-tab="sessions">项目与会话 <b>${report.summary.sessions}</b></button>
      <button class="tab ${activeTab === "orphans" ? "active" : ""}" data-tab="orphans">已删除会话的遗留记录 <b>${report.summary.orphanRecords}</b></button>
      <button class="tab ${activeTab === "history" ? "active" : ""}" data-tab="history">修复历史 <b>${repairHistory.length}</b></button>
      <button class="tab ${activeTab === "delete-history" ? "active" : ""}" data-tab="delete-history">删除历史 <b>${deleteHistory.length}</b></button>
      <button class="tab ${activeTab === "backup-history" ? "active" : ""}" data-tab="backup-history">备份历史 <b>${backupHistory.length}</b></button>
    </nav>
    ${activeTab === "sessions" ? sessionsView(filtered) : activeTab === "orphans" ? orphanView() : activeTab === "history" ? historyView() : activeTab === "delete-history" ? deleteHistoryView() : backupHistoryView()}
    ${selectedSession ? repairDialog(selectedSession) : ""}
    ${showBackupSettings ? backupSettingsDialog() : ""}
    ${showExportDialog ? exportDialog() : ""}
    ${importPackage ? importDialog(importPackage) : ""}
    ${notice ? noticeDialog(notice) : ""}
    ${pendingDeletion ? deleteConfirmationDialog(pendingDeletion) : ""}
    <div id="toast" class="toast" role="status"></div>
  `;
  bindEvents();
}

function metric(label: string, value: number, tone: string): string {
  return `<article class="metric ${tone}"><span>${label}</span><strong>${value}</strong></article>`;
}

function sessionsView(sessions: Session[]): string {
  const healthSessions = filterByHealth(report!.sessions);
  const projectSessionCount = (projectId: string) => healthSessions.filter((session) => session.projectId === projectId).length;
  const visibleProjects = report!.projects.filter((project) => healthFilter === "all" || projectSessionCount(project.id) > 0);
  const projectButtons = [
    `<button class="filter ${selectedProject === "all" ? "selected" : ""}" data-project="all">全部 <span>${healthSessions.length}</span></button>`,
    ...visibleProjects.map((project) => `<button class="filter ${selectedProject === project.id ? "selected" : ""}" data-project="${escapeHtml(project.id)}">${escapeHtml(project.name)} <span>${projectSessionCount(project.id)}</span>${healthFilter === "all" && project.issueCount ? `<i>${project.issueCount}</i>` : ""}</button>`),
  ].join("");
  const rows = sessions.map((session) => `
    <tr>
      <td class="session-cell"><strong title="${escapeHtml(session.title)}">${escapeHtml(displayTitle(session.title))}</strong><small>${escapeHtml(session.id)}</small></td>
      <td>${session.projectName ? escapeHtml(session.projectName) : '<span class="muted">未归属</span>'}</td>
      <td>${pathCell(session.projectRoots.join("\n") || null)}</td>
      <td>${pathCell(session.databaseCwd)}</td>
      <td>${pathCell(session.conversationCwd)}</td>
      <td><span class="status ${session.status}">${statusLabel(session.status)}</span><small class="status-hint">${escapeHtml(statusHint(session))}</small></td>
      <td><div class="row-actions">
        <button class="button danger" data-delete-session="${escapeHtml(session.id)}">删除</button>
        ${session.status === "match" ? "" : `<button class="button repair" data-repair="${escapeHtml(session.id)}">修复</button>`}
      </div></td>
    </tr>`).join("");
  return `
    <section class="workspace">
      <aside class="project-list"><div class="side-title">Codex 项目</div>
        <div class="health-filters" aria-label="筛选会话状态">
          ${healthFilterButton("all", "全部", report!.summary.sessions)}
          ${healthFilterButton("mismatch", "路径不一致", report!.summary.mismatched)}
          ${healthFilterButton("missing_log", "缺少对话日志", report!.summary.missingLogs)}
          ${healthFilterButton("unlinked", "未归属项目", report!.summary.unlinked)}
          ${healthFilterButton("healthy", "正常", report!.summary.matched)}
        </div><div class="project-filters">${projectButtons}</div>
        <div class="sidebar-author"><span>作者：H-Knight</span><button id="github-profile" class="github-link" type="button" aria-label="打开 H-Knight 的 GitHub 主页" title="GitHub · HaoKnight"><svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M12 .7a11.3 11.3 0 0 0-3.57 22c.57.1.77-.25.77-.55v-2.16c-3.14.68-3.8-1.34-3.8-1.34-.51-1.3-1.25-1.65-1.25-1.65-1.03-.7.08-.69.08-.69 1.13.08 1.73 1.16 1.73 1.16 1.01 1.73 2.65 1.23 3.3.94.1-.73.4-1.23.72-1.52-2.5-.29-5.14-1.25-5.14-5.59 0-1.23.44-2.24 1.16-3.03-.12-.29-.5-1.44.11-2.99 0 0 .95-.3 3.1 1.16A10.8 10.8 0 0 1 12 6.06c.96 0 1.92.13 2.82.38 2.16-1.46 3.1-1.16 3.1-1.16.62 1.55.23 2.7.12 2.99.72.79 1.16 1.8 1.16 3.03 0 4.35-2.64 5.3-5.16 5.58.41.35.77 1.04.77 2.1v3.17c0 .3.2.66.78.55A11.3 11.3 0 0 0 12 .7Z"/></svg></button></div>
      </aside>
      <div class="table-wrap">
        <div class="table-title"><div><h2>会话路径核对</h2><p>绿色：三处路径完全相同。红色：可选择一个现有目录同步到数据库和对应会话日志。</p></div><span>${sessions.length} 个会话</span></div>
        <table><thead><tr><th>会话</th><th>项目</th><th>项目目录</th><th>数据库工作目录</th><th>对话工作目录</th><th>结果</th><th></th></tr></thead>
        <tbody>${rows || `<tr><td colspan="7" class="empty-row">这个项目没有会话。</td></tr>`}</tbody></table>
      </div>
    </section>`;
}

function pathCell(path: string | null): string {
  return path ? `<code class="path" title="${escapeHtml(path)}">${escapeHtml(path)}</code>` : '<span class="muted">未找到</span>';
}

function orphanView(): string {
  const records = report!.orphans.map((record) => `
    <tr>
      <td><code class="id">${escapeHtml(record.id)}</code></td>
      <td>${pathCell(record.conversationCwd)}</td>
      <td>${pathCell(record.logPath)}</td>
      <td>${record.archived ? "归档目录" : "活动会话目录"}</td>
      <td><button class="button danger" data-delete="${escapeHtml(record.logPath)}">删除记录</button></td>
    </tr>`).join("");
  return `<section class="orphan-panel"><div class="table-title"><div><h2>未被当前 Codex 数据库引用的 JSONL 会话</h2><p>这表示会话已经从当前 Codex 列表中移除，但原始日志仍在 <code>~/.codex/sessions</code> 或 <code>archived_sessions</code>。删除前会自动备份。</p></div></div>
    <table><thead><tr><th>会话 ID</th><th>日志中的工作目录</th><th>日志文件</th><th>位置</th><th></th></tr></thead><tbody>${records || `<tr><td colspan="5" class="empty-row">没有检测到遗留记录。</td></tr>`}</tbody></table>
  </section>`;
}

function historyView(): string {
  const rows = repairHistory.map((item) => `
    <tr>
      <td><strong>${escapeHtml(displayTitle(item.sessionTitle))}</strong><small>${escapeHtml(item.threadId)}</small></td>
      <td>${escapeHtml(new Date(item.createdAt).toLocaleString("zh-CN"))}${item.rolledBackAt ? `<span class="rollback-mark">已回退</span><small>${escapeHtml(new Date(item.rolledBackAt).toLocaleString("zh-CN"))}</small>` : ""}</td>
      <td>${pathCell(item.sourceCwd)}</td>
      <td>${pathCell(item.targetCwd)}</td>
      <td>${item.fileCount} 个文件<small>${escapeHtml(item.backupFolder)}</small></td>
      <td><div class="history-actions"><button class="button repair" data-rollback="${escapeHtml(item.manifestPath)}" title="回退此次修复">回退</button><button class="button danger" data-delete-backup="${escapeHtml(item.backupFolder)}" title="清除本历史及其备份">清除</button></div></td>
    </tr>`).join("");
  return `<section class="orphan-panel history-panel"><div class="table-title"><div><h2>会话修复历史</h2><p>记录每次修复前保存的数据库和对话日志。回退前必须关闭 Codex，管理器还会额外备份当前状态。</p></div><div class="table-title-actions"><span>${repairHistory.length} 条记录</span><button class="button danger compact" data-clear-history="repair" ${repairHistory.length ? "" : "disabled"}>一键清除</button></div></div>
    <table><thead><tr><th>会话</th><th>修复时间</th><th>修复前目录</th><th>修复后目录</th><th>备份</th><th></th></tr></thead>
    <tbody>${rows || `<tr><td colspan="6" class="empty-row">当前备份目录中还没有可回退的修复记录。</td></tr>`}</tbody></table>
  </section>`;
}

function deleteHistoryView(): string {
  const rows = deleteHistory.map((item) => {
    const type = item.deletionKind === "session" ? "会话" : "遗留日志";
    const detail = item.threadId || item.sourcePath || "—";
    const children = item.childCount ? `包含 ${item.childCount} 个子代理` : "无子代理";
    return `<tr>
      <td><span class="deletion-kind">${type}</span></td>
      <td><strong>${escapeHtml(displayTitle(item.sessionTitle))}</strong><small>${escapeHtml(detail)}</small></td>
      <td>${escapeHtml(new Date(item.createdAt).toLocaleString("zh-CN"))}${item.rolledBackAt ? `<span class="rollback-mark">已回退</span><small>${escapeHtml(new Date(item.rolledBackAt).toLocaleString("zh-CN"))}</small>` : ""}</td>
      <td>${item.deletionKind === "session" ? children : "单个日志文件"}</td>
      <td>${item.fileCount} 个文件<small>${escapeHtml(item.backupFolder)}</small></td>
      <td><div class="history-actions"><button class="button repair" data-delete-rollback="${escapeHtml(`${item.backupFolder}/delete-history.json`)}" title="回退此次删除">回退</button><button class="button danger" data-delete-backup="${escapeHtml(item.backupFolder)}" title="清除本历史及其备份">清除</button></div></td>
    </tr>`;
  }).join("");
  return `<section class="orphan-panel delete-history-panel"><div class="table-title"><div><h2>删除历史</h2><p>仅显示已经成功完成的删除操作。这里保留目标信息和备份位置，不会自动覆盖当前 Codex 数据。</p></div><div class="table-title-actions"><span>${deleteHistory.length} 条记录</span><button class="button danger compact" data-clear-history="delete" ${deleteHistory.length ? "" : "disabled"}>一键清除</button></div></div>
    <table><thead><tr><th>类型</th><th>删除目标</th><th>删除时间</th><th>范围</th><th>备份</th><th></th></tr></thead>
    <tbody>${rows || `<tr><td colspan="6" class="empty-row">当前备份目录中还没有删除历史。</td></tr>`}</tbody></table>
  </section>`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function backupHistoryView(): string {
  const rows = backupHistory.map((item) => `<tr>
    <td><strong>${escapeHtml(item.operation)}</strong><small>${escapeHtml(item.name)}</small></td>
    <td>${escapeHtml(new Date(item.createdAt).toLocaleString("zh-CN"))}</td>
    <td>${item.fileCount} 个文件<small>${formatBytes(item.totalSize)}</small></td>
    <td>${pathCell(item.folder)}</td>
    <td><button class="button danger" data-delete-backup="${escapeHtml(item.folder)}" title="永久删除这份备份">删除</button></td>
  </tr>`).join("");
  return `<section class="orphan-panel backup-history-panel"><div class="table-title"><div><h2>备份历史</h2><p>集中管理每次修复、删除和回退前创建的备份。删除备份后，对应修复或删除历史将不再可回退。</p></div><div class="table-title-actions"><span>${backupHistory.length} 份备份</span><button class="button danger compact" data-clear-history="backup" ${backupHistory.length ? "" : "disabled"}>一键清除</button></div></div>
    <table><thead><tr><th>操作</th><th>创建时间</th><th>内容</th><th>位置</th><th></th></tr></thead>
    <tbody>${rows || `<tr><td colspan="5" class="empty-row">当前备份目录中还没有备份。</td></tr>`}</tbody></table>
  </section>`;
}

function repairDialog(session: Session): string {
  const defaultTarget = session.projectRoots[0] || session.databaseCwd || session.conversationCwd || "";
  return `<div class="modal-backdrop"><section class="modal" role="dialog" aria-modal="true"><button id="close-modal" class="icon-button" aria-label="关闭">×</button>
    <p class="eyebrow">修复会话路径</p><h2>${escapeHtml(displayTitle(session.title))}</h2>
    <p>目标目录会写入 Codex 数据库，并替换该会话 JSONL 中与原工作目录完全相同的字段。请先退出 Codex，避免运行中的程序把旧值写回。</p>
    <label>目标目录<div class="path-picker"><input id="target-path" value="${escapeHtml(defaultTarget)}" readonly spellcheck="false" /><button id="choose-target" class="button secondary" type="button">选择文件夹</button></div></label>
    <label class="child-agent-option"><input id="include-child-agents" type="checkbox" /> <span><strong>同时修复子代理会话</strong><small>递归更新该主会话派生的子代理数据库路径与 JSONL，并纳入同一次备份和回退。</small></span></label>
    <p class="confirmation-note">核查目标目录是否正确再点击“备份并修复”，系统会再次提示是否修复。</p>
    <div class="modal-actions"><button id="close-modal-2" class="button secondary">取消</button><button id="confirm-repair" class="button">备份并修复</button></div>
  </section></div>`;
}

function currentBackupBase(): string {
  return backupBase || report?.defaultBackupDirectory || "";
}

function backupSettingsDialog(): string {
  const backupPath = currentBackupBase();
  return `<div class="modal-backdrop"><section class="modal backup-modal" role="dialog" aria-modal="true"><button id="close-backup-settings" class="icon-button" aria-label="关闭">×</button>
    <p class="eyebrow">备份位置</p><h2>选择备份目录</h2>
    <p>每次修复或删除前，管理器会在该目录中新建一个带时间戳的备份文件夹。</p>
    <label>当前备份目录<div class="path-picker"><input id="backup-base-path" value="${escapeHtml(backupPath)}" spellcheck="false" /><button id="choose-backup" class="button secondary" type="button">选择文件夹</button></div></label>
    <p class="confirmation-note">输入不存在的绝对路径时，会在保存或首次备份时自动创建该目录。</p>
    <div class="modal-actions"><button id="open-backup" class="button secondary">打开备份目录</button><button id="save-backup-settings" class="button">保存</button></div>
  </section></div>`;
}

function exportDialog(): string {
  const groups = report!.projects.map((project) => ({
    id: project.id,
    name: project.name,
    sessions: report!.sessions.filter((session) => session.projectId === project.id),
  })).filter((group) => group.sessions.length > 0);
  const unlinked = report!.sessions.filter((session) => !session.projectId);
  if (unlinked.length) groups.push({ id: "unlinked", name: "未归属项目", sessions: unlinked });
  const visibleGroups = exportProject === "all"
    ? groups
    : groups.filter((group) => group.id === exportProject);
  const visibleSessions = visibleGroups.flatMap((group) => group.sessions);
  const projectButton = (id: string, name: string, sessions: Session[]) => {
    const checked = sessions.length > 0 && sessions.every((session) => exportSelection.has(session.id));
    return `<div class="export-project-option"><input type="checkbox" data-export-project-checkbox="${escapeHtml(id)}" ${checked ? "checked" : ""} aria-label="选择项目 ${escapeHtml(name)}" /><button class="filter ${exportProject === id ? "selected" : ""}" data-export-filter="${escapeHtml(id)}">${escapeHtml(name)} <span>${sessions.length}</span></button></div>`;
  };
  const projectButtons = [
    projectButton("all", "全部", report!.sessions),
    ...groups.map((group) => projectButton(group.id, group.name, group.sessions)),
  ].join("");
  const sessionRow = (session: Session) => `<tr>
    <td class="export-check"><input type="checkbox" data-export-session="${escapeHtml(session.id)}" ${exportSelection.has(session.id) ? "checked" : ""} aria-label="选择 ${escapeHtml(session.title)}" /></td>
    <td class="session-cell"><strong title="${escapeHtml(session.title)}">${escapeHtml(displayTitle(session.title))}</strong><small>${escapeHtml(session.id)}</small></td>
    <td>${pathCell(session.projectRoots.join("\n") || null)}</td>
    <td>${pathCell(session.databaseCwd)}</td>
    <td><span class="status ${session.status}">${statusLabel(session.status)}</span></td>
  </tr>`;
  const rows = visibleGroups.map((group) => {
    const collapsed = collapsedExportProjects.has(group.id);
    return `<tr class="export-project-heading ${collapsed ? "collapsed" : ""}" data-export-project-toggle="${escapeHtml(group.id)}" role="button" tabindex="0" aria-expanded="${!collapsed}" aria-label="${collapsed ? "展开" : "折叠"}项目 ${escapeHtml(group.name)}">
      <td colspan="5"><span class="export-collapse-indicator" aria-hidden="true">${collapsed ? "▸" : "▾"}</span><strong>${escapeHtml(group.name)}</strong><span>${group.sessions.length} 个会话</span></td>
    </tr>${collapsed ? "" : group.sessions.map(sessionRow).join("")}`;
  }).join("");
  return `<div class="modal-backdrop"><section class="modal export-modal" role="dialog" aria-modal="true"><button id="close-export" class="icon-button" aria-label="关闭">×</button>
    <h2>选择导出的项目会话</h2>
    <section class="export-workspace">
      <aside class="project-list export-project-list"><div class="side-title">Codex 项目</div><div class="project-filters">${projectButtons}</div></aside>
      <div class="export-table-panel">
        <div class="export-collapse-toolbar"><div><button id="export-collapse-all" class="button secondary compact" type="button" ${visibleGroups.length ? "" : "disabled"}>全部折叠</button><button id="export-expand-all" class="button secondary compact" type="button" ${visibleGroups.length ? "" : "disabled"}>全部展开</button></div><span>共 ${visibleGroups.length} 个项目，${visibleSessions.length} 条会话</span></div>
        <div class="table-wrap export-table-wrap">
          <table><thead><tr><th class="export-check"></th><th>会话</th><th>项目目录</th><th>数据库工作目录</th><th>状态</th></tr></thead><tbody>${rows || `<tr><td colspan="5" class="empty-row">这个项目没有可导出的会话。</td></tr>`}</tbody></table>
        </div>
      </div>
    </section>
    <div class="export-footer"><div class="export-location"><span>导出到文件夹</span><div class="path-picker"><input id="export-directory" value="${escapeHtml(exportDirectory)}" readonly placeholder="请选择保存导出包的位置" /><button id="choose-export-directory" class="button secondary" type="button">选择文件夹</button></div></div>
      <div class="modal-actions"><button id="cancel-export" class="button secondary">取消</button><button id="confirm-export" class="button" ${exportSelection.size && exportDirectory ? "" : "disabled"}>导出 ${exportSelection.size} 个会话</button></div>
    </div>
  </section></div>`;
}

function importDialog(pkg: ImportPackageInfo): string {
  const projectNames = pkg.projects.map((project) => project.name).join("、") || "未归属项目";
  return `<div class="modal-backdrop"><section class="modal import-modal" role="dialog" aria-modal="true"><button id="close-import" class="icon-button" aria-label="关闭">×</button>
    <p class="eyebrow">导入会话</p><h2>确认导入会话包</h2>
    <p>导出时间：${escapeHtml(new Date(pkg.exportedAt).toLocaleString("zh-CN"))}</p>
    <p>项目：${escapeHtml(projectNames)}<br />主会话：${pkg.visibleThreadCount} 个；包含会话记录：${pkg.threadCount} 个；对话日志：${pkg.logCount} 个。</p>
    <label>导出包清单<input value="${escapeHtml(pkg.manifestPath)}" readonly /></label>
    <p class="confirmation-note">导入会写入 Codex 状态数据库、项目目录配置、侧栏项目归属和会话日志。原工作目录会保留，导入后可用本软件选择本机目录进行路径修复。导入前必须关闭 Codex。</p>
    <div class="modal-actions"><button id="cancel-import" class="button secondary">取消</button><button id="confirm-import" class="button">备份并导入</button></div>
  </section></div>`;
}

function noticeDialog(currentNotice: Notice): string {
  const action = currentNotice.canCloseCodex
    ? `<button id="close-codex-processes" class="button danger">一键关闭 Codex 进程</button>`
    : "";
  return `<div class="modal-backdrop notice-backdrop"><section class="modal notice-modal ${currentNotice.tone}" role="alertdialog" aria-modal="true">
    <p class="eyebrow">${currentNotice.tone === "success" ? "操作完成" : "需要处理"}</p><h2>${escapeHtml(currentNotice.title)}</h2>
    <p class="notice-message">${escapeHtml(currentNotice.message)}</p>
    <div class="modal-actions">${action}<button id="close-notice" class="button ${currentNotice.tone === "warning" ? "secondary" : ""}">确定</button></div>
  </section></div>`;
}

function deleteConfirmationDialog(pending: PendingDeletion): string {
  const target = pending.kind === "session"
    ? `会话“${displayTitle(pending.session.title)}”及其全部子代理`
    : `遗留日志“${pending.logPath}”`;
  return `<div class="modal-backdrop delete-confirmation-backdrop"><section class="modal delete-confirmation-modal" role="alertdialog" aria-modal="true" aria-labelledby="delete-confirmation-title">
    <button id="close-delete-confirmation" class="icon-button" aria-label="关闭">×</button>
    <p class="eyebrow danger-eyebrow">永久删除</p><h2 id="delete-confirmation-title">确认删除 ${escapeHtml(target)}</h2>
    <p>这是第二次确认。继续后，管理器会先完成备份，再通过 Codex 自带接口永久删除该记录。</p>
    <p class="confirmation-note">该操作不可撤销；如需保留会话，请点击取消。</p>
    <div class="modal-actions"><button id="cancel-delete-confirmation" class="button secondary">取消</button><button id="confirm-delete-action" class="button danger">确认永久删除</button></div>
  </section></div>`;
}

function bindEvents(): void {
  document.querySelector("#rescan")?.addEventListener("click", () => load(true));
  document.querySelector("#theme-toggle")?.addEventListener("click", () => {
    theme = theme === "dark" ? "light" : "dark";
    localStorage.setItem("theme", theme);
    render();
  });
  document.querySelector("#backup-settings")?.addEventListener("click", () => { showBackupSettings = true; render(); });
  document.querySelector("#export-sessions")?.addEventListener("click", () => {
    collapsedExportProjects.clear();
    showExportDialog = true;
    render();
  });
  document.querySelector("#import-sessions")?.addEventListener("click", chooseImportPackage);
  document.querySelector("#github-profile")?.addEventListener("click", () => openUrl("https://github.com/HaoKnight"));
  document.querySelectorAll<HTMLButtonElement>("[data-tab]").forEach((button) => button.addEventListener("click", () => {
    activeTab = button.dataset.tab as typeof activeTab; selectedSession = null; render();
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-project]").forEach((button) => button.addEventListener("click", () => {
    selectedProject = button.dataset.project!; render();
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-health]").forEach((button) => button.addEventListener("click", () => {
    healthFilter = button.dataset.health as typeof healthFilter;
    selectedProject = "all";
    render();
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-repair]").forEach((button) => button.addEventListener("click", () => {
    selectedSession = report!.sessions.find((session) => session.id === button.dataset.repair) ?? null; render();
  }));
  document.querySelector("#close-modal")?.addEventListener("click", () => { selectedSession = null; render(); });
  document.querySelector("#close-modal-2")?.addEventListener("click", () => { selectedSession = null; render(); });
  document.querySelector("#choose-target")?.addEventListener("click", chooseTargetDirectory);
  document.querySelector("#confirm-repair")?.addEventListener("click", repairSelected);
  document.querySelector("#close-backup-settings")?.addEventListener("click", () => { showBackupSettings = false; render(); });
  document.querySelector("#save-backup-settings")?.addEventListener("click", saveBackupDirectory);
  document.querySelector("#choose-backup")?.addEventListener("click", chooseBackupDirectory);
  document.querySelector("#open-backup")?.addEventListener("click", openBackupDirectory);
  document.querySelector("#close-export")?.addEventListener("click", closeExportDialog);
  document.querySelector("#cancel-export")?.addEventListener("click", closeExportDialog);
  document.querySelector("#choose-export-directory")?.addEventListener("click", chooseExportDirectory);
  document.querySelector("#confirm-export")?.addEventListener("click", exportSelectedSessions);
  const toggleExportProject = (projectId: string) => {
    if (collapsedExportProjects.has(projectId)) collapsedExportProjects.delete(projectId);
    else collapsedExportProjects.add(projectId);
    render();
  };
  document.querySelectorAll<HTMLTableRowElement>("[data-export-project-toggle]").forEach((row) => {
    row.addEventListener("click", () => toggleExportProject(row.dataset.exportProjectToggle!));
    row.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      toggleExportProject(row.dataset.exportProjectToggle!);
    });
  });
  document.querySelector("#export-collapse-all")?.addEventListener("click", () => {
    exportProjectGroups().forEach((group) => collapsedExportProjects.add(group.id));
    render();
  });
  document.querySelector("#export-expand-all")?.addEventListener("click", () => {
    collapsedExportProjects.clear();
    render();
  });
  document.querySelectorAll<HTMLInputElement>("[data-export-session]").forEach((input) => input.addEventListener("change", () => {
    const id = input.dataset.exportSession!;
    if (input.checked) exportSelection.add(id); else exportSelection.delete(id);
    render();
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-export-filter]").forEach((button) => button.addEventListener("click", () => {
    exportProject = button.dataset.exportFilter!;
    render();
  }));
  document.querySelectorAll<HTMLInputElement>("[data-export-project-checkbox]").forEach((input) => input.addEventListener("change", () => {
    const projectId = input.dataset.exportProjectCheckbox!;
    exportSessionsForProject(projectId).forEach((session) => { if (input.checked) exportSelection.add(session.id); else exportSelection.delete(session.id); });
    render();
  }));
  document.querySelector("#close-import")?.addEventListener("click", () => { importPackage = null; render(); });
  document.querySelector("#cancel-import")?.addEventListener("click", () => { importPackage = null; render(); });
  document.querySelector("#confirm-import")?.addEventListener("click", importSelectedPackage);
  document.querySelector("#close-notice")?.addEventListener("click", () => { notice = null; render(); });
  document.querySelector("#close-codex-processes")?.addEventListener("click", closeCodexProcesses);
  document.querySelector("#close-delete-confirmation")?.addEventListener("click", closeDeleteConfirmation);
  document.querySelector("#cancel-delete-confirmation")?.addEventListener("click", closeDeleteConfirmation);
  document.querySelector("#confirm-delete-action")?.addEventListener("click", executePendingDeletion);
  document.querySelectorAll<HTMLButtonElement>("[data-delete]").forEach((button) => button.addEventListener("click", () => deleteOrphan(button.dataset.delete!)));
  document.querySelectorAll<HTMLButtonElement>("[data-delete-session]").forEach((button) => button.addEventListener("click", () => {
    const session = report!.sessions.find((item) => item.id === button.dataset.deleteSession);
    if (session) void deleteSession(session);
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-rollback]").forEach((button) => button.addEventListener("click", () => rollbackRepair(button.dataset.rollback!)));
  document.querySelectorAll<HTMLButtonElement>("[data-delete-rollback]").forEach((button) => button.addEventListener("click", () => rollbackDelete(button.dataset.deleteRollback!)));
  document.querySelectorAll<HTMLButtonElement>("[data-delete-backup]").forEach((button) => button.addEventListener("click", () => deleteBackup(button.dataset.deleteBackup!)));
  document.querySelectorAll<HTMLButtonElement>("[data-clear-history]").forEach((button) => button.addEventListener("click", () => clearHistory(button.dataset.clearHistory!)));
  document.querySelectorAll<HTMLInputElement>("[data-export-project-checkbox]").forEach((input) => {
    const sessions = exportSessionsForProject(input.dataset.exportProjectCheckbox!);
    const selected = sessions.filter((session) => exportSelection.has(session.id)).length;
    input.indeterminate = selected > 0 && selected < sessions.length;
  });
}

function exportSessionsForProject(projectId: string): Session[] {
  if (projectId === "all") return report?.sessions ?? [];
  return (report?.sessions ?? []).filter((session) => projectId === "unlinked" ? !session.projectId : session.projectId === projectId);
}

function exportProjectGroups(): Array<{ id: string }> {
  const groups = (report?.projects ?? [])
    .filter((project) => (report?.sessions ?? []).some((session) => session.projectId === project.id))
    .map((project) => ({ id: project.id }));
  if ((report?.sessions ?? []).some((session) => !session.projectId)) groups.push({ id: "unlinked" });
  return groups;
}

async function load(showSuccess = false): Promise<void> {
  try {
    const cleanedSidebarReferences = await invoke<number>("cleanup_deleted_sidebar_references", { backupBase: currentBackupBase() });
    report = await invoke<AuditReport>("scan_codex");
    if (!backupBase) backupBase = report.defaultBackupDirectory;
    [repairHistory, deleteHistory, backupHistory] = await Promise.all([
      invoke<RepairHistoryItem[]>("list_repair_history", { backupBase: currentBackupBase() }),
      invoke<DeleteHistoryItem[]>("list_delete_history", { backupBase: currentBackupBase() }),
      invoke<BackupHistoryItem[]>("list_backup_history", { backupBase: currentBackupBase() }),
    ]);
    render();
    const cleanupMessage = cleanedSidebarReferences > 0 ? `已清理 ${cleanedSidebarReferences} 条侧栏引用；请重启 Codex 刷新侧边栏。` : "";
    if (showSuccess) showToast(`重新扫描完成：发现 ${report.summary.sessions} 个会话、${report.summary.sessions - report.summary.matched} 个问题会话。${cleanupMessage ? ` ${cleanupMessage}` : ""}`);
    else if (cleanupMessage) showToast(cleanupMessage);
  } catch (error) {
    app.innerHTML = `<section class="empty error"><h1>无法扫描 Codex 数据</h1><p>${escapeHtml(String(error))}</p><button id="rescan" class="button">重试</button></section>`;
    document.querySelector("#rescan")?.addEventListener("click", () => load(true));
  }
}

function closeExportDialog(): void {
  showExportDialog = false;
  exportSelection.clear();
  exportDirectory = "";
  exportProject = "all";
  collapsedExportProjects.clear();
  render();
}

async function chooseExportDirectory(): Promise<void> {
  const selection = await open({
    title: "选择导出会话包的保存位置",
    directory: true,
    multiple: false,
    defaultPath: exportDirectory || undefined,
  });
  if (typeof selection === "string") {
    exportDirectory = selection;
    render();
  }
}

async function exportSelectedSessions(): Promise<void> {
  if (!exportSelection.size || !exportDirectory) return;
  const button = document.querySelector<HTMLButtonElement>("#confirm-export");
  if (button) { button.disabled = true; button.textContent = "导出中…"; }
  try {
    const result = await invoke<ActionResult>("export_sessions", {
      request: { threadIds: [...exportSelection], destinationDirectory: exportDirectory },
    });
    showExportDialog = false;
    exportSelection.clear();
    exportDirectory = "";
    notice = { title: "会话导出成功", message: `${result.message}\n\n导出内容：\n${result.changes.map((change) => `• ${change}`).join("\n")}\n\n导出包位置：${result.backupFolder}`, tone: "success" };
    render();
  } catch (error) {
    showToast(String(error), true);
    if (button) { button.disabled = false; button.textContent = `导出 ${exportSelection.size} 个会话`; }
  }
}

async function chooseImportPackage(): Promise<void> {
  try {
    const selection = await open({
      title: "选择 Codex 会话导出包中的 manifest.json",
      directory: false,
      multiple: false,
      filters: [{ name: "会话导出清单", extensions: ["json"] }],
    });
    if (typeof selection !== "string") return;
    importPackage = await invoke<ImportPackageInfo>("inspect_export_package", { manifestPath: selection });
    render();
  } catch (error) { showToast(String(error), true); }
}

async function importSelectedPackage(): Promise<void> {
  if (!importPackage) return;
  const approved = await confirm(
    `将导入 ${importPackage.visibleThreadCount} 个主会话及其配置，并把记录加入本机 Codex。\n\n导入前会备份当前状态；原工作目录将保留，之后可在本软件中修复路径。确认继续吗？`,
    { title: "确认导入会话", kind: "warning", okLabel: "备份并导入", cancelLabel: "取消" },
  );
  if (!approved) return;
  try {
    const result = await invoke<ActionResult>("import_sessions", {
      request: { manifestPath: importPackage.manifestPath, confirmation: "IMPORT", backupBase: currentBackupBase() },
    });
    importPackage = null;
    await load();
    notice = { title: "会话导入成功", message: `${result.message}\n\n导入内容：\n${result.changes.map((change) => `• ${change}`).join("\n")}\n\n导入前备份：${result.backupFolder}`, tone: "success" };
    render();
  } catch (error) {
    const text = String(error);
    if (text.includes("Codex 进程仍在运行")) {
      importPackage = null;
      notice = { title: "请先退出 Codex", message: text, tone: "warning", canCloseCodex: true };
      render();
      return;
    }
    showToast(text, true);
  }
}

async function repairSelected(): Promise<void> {
  if (!selectedSession) return;
  const target = document.querySelector<HTMLInputElement>("#target-path")!.value;
  const includeChildAgents = document.querySelector<HTMLInputElement>("#include-child-agents")?.checked ?? false;
  if (!target) { showToast("请先选择目标文件夹。", true); return; }
  const scope = includeChildAgents ? "该会话及其全部子代理" : "该会话";
  const approved = await confirm(
    `将把${scope}的数据库和对话记录工作目录同步为：\n${target}\n\n操作前会自动备份。确认继续吗？`,
    { title: "确认修复会话路径", kind: "warning", okLabel: "备份并修复", cancelLabel: "取消" },
  );
  if (!approved) return;
  try {
    const result = await invoke<ActionResult>("repair_session", { request: { threadId: selectedSession.id, targetPath: target, confirmation: "REPAIR", backupBase: currentBackupBase(), includeChildAgents } });
    selectedSession = null;
    await load();
    notice = { title: "修复成功", message: `${result.message}\n\n修复内容：\n${result.changes.map((change) => `• ${change}`).join("\n")}\n\n备份位置：${result.backupFolder}`, tone: "success" };
    render();
  } catch (error) {
    const text = String(error);
    if (text.includes("Codex 进程仍在运行")) {
      selectedSession = null;
      notice = { title: "请先退出 Codex", message: text, tone: "warning", canCloseCodex: true };
      render();
      return;
    }
    showToast(text, true);
  }
}

async function closeCodexProcesses(): Promise<void> {
  try {
    const result = await invoke<ProcessCloseResult>("close_codex_processes");
    notice = { title: "已请求关闭 Codex", message: result.message, tone: "success" };
    render();
  } catch (error) { showToast(String(error), true); }
}

async function rollbackRepair(manifestPath: string): Promise<void> {
  const item = repairHistory.find((history) => history.manifestPath === manifestPath);
  if (!item) return;
  const approved = await confirm(
    `将把“${item.sessionTitle}”恢复到修复前的目录：\n${item.sourceCwd}\n\n回退前会先备份当前状态。确认继续吗？`,
    { title: "确认回退修复", kind: "warning", okLabel: "备份并回退", cancelLabel: "取消" },
  );
  if (!approved) return;
  try {
    const result = await invoke<ActionResult>("rollback_repair", { request: { manifestPath, backupBase: currentBackupBase(), confirmation: "ROLLBACK" } });
    await load();
    notice = { title: "回退成功", message: `${result.message}\n\n回退内容：\n${result.changes.map((change) => `• ${change}`).join("\n")}\n\n回退前备份：${result.backupFolder}`, tone: "success" };
    render();
  } catch (error) {
    const text = String(error);
    if (text.includes("Codex 进程仍在运行")) {
      notice = { title: "请先退出 Codex", message: text, tone: "warning", canCloseCodex: true };
      render();
      return;
    }
    showToast(text, true);
  }
}

async function rollbackDelete(manifestPath: string): Promise<void> {
  const item = deleteHistory.find((history) => `${history.backupFolder}/delete-history.json` === manifestPath);
  if (!item) return;
  const approved = await confirm(
    `将恢复“${displayTitle(item.sessionTitle)}”${item.childCount ? `及其 ${item.childCount} 个子代理` : ""}。\n\n回退前会先备份当前状态，且必须先退出 Codex。确认继续吗？`,
    { title: "确认回退删除", kind: "warning", okLabel: "备份并恢复", cancelLabel: "取消" },
  );
  if (!approved) return;
  try {
    const result = await invoke<ActionResult>("rollback_delete", { request: { manifestPath, backupBase: currentBackupBase(), confirmation: "RESTORE_DELETION" } });
    await load();
    notice = { title: "删除已回退", message: `${result.message}\n\n恢复内容：\n${result.changes.map((change) => `• ${change}`).join("\n")}\n\n回退前备份：${result.backupFolder}`, tone: "success" };
    render();
  } catch (error) {
    const text = String(error);
    if (text.includes("Codex 进程仍在运行")) {
      notice = { title: "请先退出 Codex", message: text, tone: "warning", canCloseCodex: true };
      render();
      return;
    }
    showToast(text, true);
  }
}

async function chooseTargetDirectory(): Promise<void> {
  const input = document.querySelector<HTMLInputElement>("#target-path");
  if (!input) return;
  const selection = await open({
    title: "选择修复后的工作目录",
    directory: true,
    multiple: false,
    defaultPath: input.value || undefined,
  });
  if (typeof selection === "string") input.value = selection;
}

async function chooseBackupDirectory(): Promise<void> {
  const selection = await open({
    title: "选择会话管理备份目录",
    directory: true,
    multiple: false,
    defaultPath: currentBackupBase() || undefined,
  });
  if (typeof selection === "string") {
    const input = document.querySelector<HTMLInputElement>("#backup-base-path");
    if (input) input.value = selection;
  }
}

async function saveBackupDirectory(): Promise<void> {
  const input = document.querySelector<HTMLInputElement>("#backup-base-path");
  const path = input?.value.trim() ?? "";
  if (!path) { showToast("请输入或选择备份目录。", true); return; }
  try {
    backupBase = await invoke<string>("prepare_backup_directory", { backupBase: path });
    localStorage.setItem("backup-base", backupBase);
    [repairHistory, deleteHistory, backupHistory] = await Promise.all([
      invoke<RepairHistoryItem[]>("list_repair_history", { backupBase }),
      invoke<DeleteHistoryItem[]>("list_delete_history", { backupBase }),
      invoke<BackupHistoryItem[]>("list_backup_history", { backupBase }),
    ]);
    showBackupSettings = false;
    render();
    showToast("备份目录已保存并确认可用。");
  } catch (error) { showToast(String(error), true); }
}

async function openBackupDirectory(): Promise<void> {
  try {
    const path = await invoke<string>("prepare_backup_directory", { backupBase: currentBackupBase() });
    await openPath(path);
  } catch (error) { showToast(String(error), true); }
}

async function deleteBackup(backupFolder: string): Promise<void> {
  const item = backupHistory.find((history) => history.folder === backupFolder);
  const description = item ? `${item.operation}备份“${item.name}”（${item.fileCount} 个文件，${formatBytes(item.totalSize)}）` : "该备份";
  const approved = await confirm(
    `将永久删除${description}。删除后，对应的修复或删除历史也无法再回退。确认继续吗？`,
    { title: "确认删除备份", kind: "warning", okLabel: "永久删除", cancelLabel: "取消" },
  );
  if (!approved) return;
  try {
    await invoke("delete_backup", { request: { backupFolder, backupBase: currentBackupBase(), confirmation: "DELETE_BACKUP" } });
    await load();
    showToast("备份及对应历史已清除。");
  } catch (error) { showToast(String(error), true); }
}

async function clearHistory(kind: string): Promise<void> {
  const count = kind === "repair" ? repairHistory.length : kind === "delete" ? deleteHistory.length : backupHistory.length;
  if (!count) { showToast("当前没有可清除的历史记录。"); return; }
  const name = kind === "repair" ? "修复历史" : kind === "delete" ? "删除历史" : "备份历史";
  const approved = await confirm(
    `将永久清除${name}中的 ${count} 条记录及其对应备份文件。此操作无法回退，确认继续吗？`,
    { title: "确认一键清除", kind: "warning", okLabel: "永久清除", cancelLabel: "取消" },
  );
  if (!approved) return;
  try {
    const removed = await invoke<number>("clear_history", { request: { kind, backupBase: currentBackupBase(), confirmation: "CLEAR_HISTORY" } });
    await load();
    showToast(`已清除 ${removed} 条${name}记录及对应备份。`);
  } catch (error) { showToast(String(error), true); }
}

async function deleteSession(session: Session): Promise<void> {
  const approved = await confirm(
    `即将永久删除会话“${displayTitle(session.title)}”及其全部子代理。\n\n为避免 Codex 侧栏留下无法恢复的缓存条目，删除前必须先退出 Codex。管理器会在 Codex 已退出后创建一致性状态数据库备份，并备份相关会话日志。确认继续吗？`,
    { title: "确认删除会话", kind: "warning", okLabel: "继续删除", cancelLabel: "取消" },
  );
  if (!approved) return;
  pendingDeletion = { kind: "session", session };
  render();
}

function closeDeleteConfirmation(): void {
  pendingDeletion = null;
  render();
}

async function executePendingDeletion(): Promise<void> {
  if (!pendingDeletion) return;
  const confirmation = "DELETE";
  const button = document.querySelector<HTMLButtonElement>("#confirm-delete-action");
  if (button) { button.disabled = true; button.textContent = "删除中…"; }
  const deletion = pendingDeletion;
  try {
    if (deletion.kind === "session") {
      const result = await invoke<ActionResult>("delete_session", { request: { threadId: deletion.session.id, confirmation, backupBase: currentBackupBase() } });
      pendingDeletion = null;
      await load();
      notice = { title: "会话已删除", message: `${result.message}\n\n备份位置：${result.backupFolder}`, tone: "success" };
      render();
    } else {
      const result = await invoke<ActionResult>("delete_orphan", { request: { logPath: deletion.logPath, confirmation, backupBase: currentBackupBase() } });
      pendingDeletion = null;
      await load();
      notice = { title: "遗留记录已删除", message: `${result.message}\n\n备份位置：${result.backupFolder}`, tone: "success" };
      render();
    }
  } catch (error) {
    const text = String(error);
    if (text.includes("Codex 进程仍在运行")) {
      pendingDeletion = null;
      notice = { title: "请先退出 Codex", message: text, tone: "warning", canCloseCodex: true };
      render();
      return;
    }
    showToast(text, true);
    if (button) { button.disabled = false; button.textContent = "确认永久删除"; }
  }
}

async function deleteOrphan(logPath: string): Promise<void> {
  pendingDeletion = { kind: "orphan", logPath };
  render();
}

function showToast(message: string, error = false): void {
  const toast = document.querySelector<HTMLDivElement>("#toast");
  if (!toast) return;
  toast.textContent = message;
  toast.className = `toast visible ${error ? "toast-error" : ""}`;
  window.setTimeout(() => { toast.className = "toast"; }, 2000);
}

render();
void load();
