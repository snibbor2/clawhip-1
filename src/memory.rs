use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use time::OffsetDateTime;

use crate::Result;
use crate::cli::{DailyFormat, HierarchyMode, MemoryAuditArgs, MemoryInitArgs, MemoryRotateArgs, MemoryStatusArgs};

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryLayout {
    root: PathBuf,
    project_slug: String,
    channel_slug: Option<String>,
    agent_slug: Option<String>,
    today_slug: String,
    deep: bool,
    daily_folder: bool,
    tags: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryInitReport {
    written_files: Vec<PathBuf>,
    skipped_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryStatusReport {
    layout: MemoryLayout,
    memory_file_exists: bool,
    memory_dir_exists: bool,
    markdown_file_count: usize,
    missing_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryAuditReport {
    pub(crate) project_slug: String,
    pub(crate) inspected_daily_slug: String,
    pub(crate) report_path: PathBuf,
    pub(crate) markdown_file_count: usize,
    pub(crate) missing_paths: Vec<PathBuf>,
    pub(crate) summary: String,
}

pub fn init(args: MemoryInitArgs) -> Result<()> {
    let force = args.force;
    let layout = MemoryLayout::from_init_args(args)?;
    let report = initialize_layout(&layout, force)?;

    println!(
        "Initialized filesystem-offloaded memory scaffold at {}",
        layout.root.display()
    );
    println!("Project slug: {}", layout.project_slug);
    println!(
        "Project partition: {}",
        layout.project_partition_index_file().display()
    );
    println!(
        "Canonical daily file: {}",
        layout.project_partition_daily_file().display()
    );
    println!("Written files: {}", report.written_files.len());
    for path in &report.written_files {
        println!("  wrote {}", display_relative(&layout.root, path));
    }
    if !report.skipped_files.is_empty() {
        println!("Skipped existing files: {}", report.skipped_files.len());
        for path in &report.skipped_files {
            println!("  kept {}", display_relative(&layout.root, path));
        }
    }

    Ok(())
}

pub fn status(args: MemoryStatusArgs) -> Result<()> {
    let layout = MemoryLayout::from_status_args(args)?;
    let report = inspect_layout(&layout)?;

    println!("Memory root: {}", report.layout.root.display());
    println!("MEMORY.md: {}", yes_no(report.memory_file_exists));
    println!("memory/: {}", yes_no(report.memory_dir_exists));
    println!(
        "Markdown files under memory/: {}",
        report.markdown_file_count
    );
    println!(
        "Project partition: {}",
        report.layout.project_partition_index_file().display()
    );
    println!(
        "Project shard pointer: {}",
        report.layout.project_file().display()
    );
    println!(
        "Daily partition: {}",
        report.layout.project_partition_daily_file().display()
    );
    println!("Daily pointer: {}", report.layout.daily_file().display());
    println!(
        "Cron audit dir: {}",
        report.layout.project_partition_cron_audit_dir().display()
    );
    if let Some(path) = report.layout.channel_file() {
        println!("Channel pointer: {}", path.display());
    }
    if let Some(path) = report.layout.project_partition_channel_file() {
        println!("Channel partition: {}", path.display());
    }
    if let Some(path) = report.layout.agent_file() {
        println!("Agent shard: {}", path.display());
    }

    if report.missing_paths.is_empty() {
        println!("Status: scaffold looks ready");
    } else {
        println!("Missing recommended paths:");
        for path in report.missing_paths {
            println!("  - {}", display_relative(&report.layout.root, &path));
        }
    }

    Ok(())
}

pub async fn audit(args: MemoryAuditArgs) -> Result<()> {
    let layout = MemoryLayout::build(
        args.root,
        args.project,
        None,
        None,
        None,
        false,
        false,
        false,
    )?;
    let issues = run_audit_checks(&layout)?;

    if issues.is_empty() {
        println!("Memory audit passed for {}: no issues found.", layout.project_slug);
    } else {
        println!(
            "Memory audit for {}: {} issue(s) found.",
            layout.project_slug,
            issues.len()
        );
        for issue in &issues {
            println!("  - {issue}");
        }
    }

    if args.fix && !issues.is_empty() {
        let fixed = apply_audit_fixes(&layout)?;
        println!("Applied {} fix(es).", fixed);
    }

    if let Some(channel) = args.report_channel {
        let summary = if issues.is_empty() {
            format!(
                "memory audit ok for {}: no issues found",
                layout.project_slug
            )
        } else {
            let issue_list = issues.join("\n- ");
            format!(
                "memory audit for {}: {} issue(s)\n- {}",
                layout.project_slug,
                issues.len(),
                issue_list,
            )
        };
        let config = crate::config::AppConfig::default();
        let client = crate::client::DaemonClient::from_config(&config);
        let event = crate::events::IncomingEvent::custom(Some(channel), summary);
        client.send_event(&event).await?;
    }

    Ok(())
}

pub fn rotate(args: MemoryRotateArgs) -> Result<()> {
    let layout = MemoryLayout::build(
        args.root,
        args.project,
        None,
        None,
        args.date,
        true,
        true,
        true,
    )?;

    let day_dir = layout.daily_day_dir();
    fs::create_dir_all(&day_dir)
        .with_context(|| format!("create daily rotation directory {}", day_dir.display()))?;

    let files = daily_folder_files(&layout);
    let mut written = 0;
    for (path, contents) in &files {
        if write_scaffold_file(path, contents, false)? {
            written += 1;
            println!("  wrote {}", display_relative(&layout.root, path));
        }
    }

    println!(
        "Rotated daily folder for {} at {}",
        layout.project_slug,
        display_relative(&layout.root, &day_dir)
    );
    println!("Files written: {written}");
    Ok(())
}

fn run_audit_checks(layout: &MemoryLayout) -> Result<Vec<String>> {
    let mut issues = Vec::new();

    // Check for stray daily files at root level (should be in daily/YYYY-MM/DD/)
    let daily_dir = layout.daily_dir();
    if daily_dir.is_dir() {
        for entry in fs::read_dir(&daily_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "md") {
                issues.push(format!(
                    "stray daily file at root: {} (should be in daily/YYYY-MM/DD/)",
                    display_relative(&layout.root, &path)
                ));
            }
        }
    }

    // Check for missing tag headers on markdown files
    let memory_dir = layout.memory_dir();
    if memory_dir.is_dir() {
        check_tag_headers(&memory_dir, &layout.root, &mut issues)?;
    }

    // Check MEMORY.md staleness (file count mismatch)
    let memory_file = layout.memory_file();
    if memory_file.is_file() {
        let content = fs::read_to_string(&memory_file)?;
        let pointer_count = content.matches("memory/").count();
        let actual_count = count_markdown_files(&memory_dir)?;
        if actual_count > 0 && pointer_count < actual_count / 2 {
            issues.push(format!(
                "MEMORY.md may be stale: {pointer_count} pointers vs {actual_count} markdown files"
            ));
        }
    } else {
        issues.push("MEMORY.md is missing".to_string());
    }

    // Check for empty directories
    for dir in layout.expected_dirs() {
        if dir.is_dir() && is_empty_dir(&dir)? {
            issues.push(format!(
                "empty directory: {}",
                display_relative(&layout.root, &dir)
            ));
        }
    }

    // Check projects missing status/current.md or decisions/log.md
    let status_current = layout.project_status_current();
    if layout.project_partition_dir().is_dir() && !status_current.exists() {
        issues.push(format!(
            "project missing status/current.md: {}",
            display_relative(&layout.root, &status_current)
        ));
    }
    let decisions_log = layout.project_decisions_log();
    if layout.project_partition_dir().is_dir() && !decisions_log.exists() {
        issues.push(format!(
            "project missing decisions/log.md: {}",
            display_relative(&layout.root, &decisions_log)
        ));
    }

    Ok(issues)
}

fn check_tag_headers(dir: &Path, root: &Path, issues: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            check_tag_headers(&path, root, issues)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            if !content.starts_with("> 태그:") && !content.starts_with("# ") {
                // Only flag files that are non-trivial (not gitkeep stubs)
            } else if !content.contains("태그:") {
                // Skip tag check for files that are clearly index/pointer files
                let filename = path.file_name().unwrap_or_default().to_string_lossy();
                if !matches!(filename.as_ref(), "README.md" | "scaffold.toml" | ".gitkeep") {
                    issues.push(format!(
                        "missing tag header: {}",
                        display_relative(root, &path)
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_empty_dir(dir: &Path) -> Result<bool> {
    Ok(fs::read_dir(dir)?.next().is_none())
}

fn apply_audit_fixes(layout: &MemoryLayout) -> Result<usize> {
    let mut fixed = 0;

    // Fix: create missing status/current.md and decisions/log.md
    if layout.project_partition_dir().is_dir() {
        let status = layout.project_status_current();
        if !status.exists() {
            fs::create_dir_all(status.parent().unwrap())?;
            fs::write(
                &status,
                format!(
                    "# current status\n\n- Active project: `{}`\n- Created by audit --fix.\n",
                    layout.project_slug
                ),
            )?;
            fixed += 1;
        }
        let decisions = layout.project_decisions_log();
        if !decisions.exists() {
            fs::create_dir_all(decisions.parent().unwrap())?;
            fs::write(
                &decisions,
                "# decisions log\n\n- Created by audit --fix.\n",
            )?;
            fixed += 1;
        }
    }

    // Fix: create MEMORY.md if missing
    if !layout.memory_file().exists() {
        fs::write(layout.memory_file(), render_memory_md(layout))?;
        fixed += 1;
    }

    Ok(fixed)
}

pub(crate) fn run_cron_audit(
    root: PathBuf,
    project: Option<String>,
    channel: Option<String>,
    agent: Option<String>,
    date: Option<String>,
    generated_at: OffsetDateTime,
) -> Result<MemoryAuditReport> {
    let layout = MemoryLayout::build(Some(root), project, channel, agent, date, false, false, false)?;
    let status = inspect_layout(&layout)?;
    let audit_day_slug = format!(
        "{:04}-{:02}-{:02}",
        generated_at.year(),
        u8::from(generated_at.month()),
        generated_at.day()
    );
    let report_path = layout.project_partition_cron_audit_file(&audit_day_slug);

    append_cron_audit_report(&status, &report_path, generated_at)?;

    let summary = if status.missing_paths.is_empty() {
        format!(
            "memory audit ok for {}: project/channel/daily partitions look ready",
            status.layout.project_slug
        )
    } else {
        format!(
            "memory audit found {} missing path(s) for {}",
            status.missing_paths.len(),
            status.layout.project_slug
        )
    };

    Ok(MemoryAuditReport {
        project_slug: status.layout.project_slug.clone(),
        inspected_daily_slug: status.layout.today_slug.clone(),
        report_path,
        markdown_file_count: status.markdown_file_count,
        missing_paths: status.missing_paths,
        summary,
    })
}

impl MemoryLayout {
    fn from_init_args(args: MemoryInitArgs) -> Result<Self> {
        let deep = matches!(args.hierarchy, HierarchyMode::Deep);
        let daily_folder = matches!(args.daily_format, DailyFormat::Folder);
        let tags = args.tags;
        Self::build(
            args.root,
            args.project,
            args.channel,
            args.agent,
            args.date,
            deep,
            daily_folder,
            tags,
        )
    }

    fn from_status_args(args: MemoryStatusArgs) -> Result<Self> {
        Self::build(
            args.root,
            args.project,
            args.channel,
            args.agent,
            args.date,
            false,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        root: Option<PathBuf>,
        project: Option<String>,
        channel: Option<String>,
        agent: Option<String>,
        date: Option<String>,
        deep: bool,
        daily_folder: bool,
        tags: bool,
    ) -> Result<Self> {
        let root = match root {
            Some(root) => root,
            None => env::current_dir().context("resolve current directory for memory root")?,
        };
        let project = project
            .or_else(|| {
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .ok_or_else(|| anyhow!("unable to infer project slug from root path"))?;

        Ok(Self {
            root,
            project_slug: slugify(&project)?,
            channel_slug: channel.map(|value| slugify(&value)).transpose()?,
            agent_slug: agent.map(|value| slugify(&value)).transpose()?,
            today_slug: normalize_date_slug(date)?,
            deep,
            daily_folder,
            tags,
        })
    }

    fn memory_file(&self) -> PathBuf {
        self.root.join("MEMORY.md")
    }

    fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    fn memory_index_file(&self) -> PathBuf {
        self.memory_dir().join("README.md")
    }

    fn scaffold_config_file(&self) -> PathBuf {
        self.memory_dir().join("scaffold.toml")
    }

    fn daily_dir(&self) -> PathBuf {
        self.memory_dir().join("daily")
    }

    fn daily_file(&self) -> PathBuf {
        self.daily_dir().join(format!("{}.md", self.today_slug))
    }

    fn projects_dir(&self) -> PathBuf {
        self.memory_dir().join("projects")
    }

    fn project_file(&self) -> PathBuf {
        self.projects_dir()
            .join(format!("{}.md", self.project_slug))
    }

    fn project_partition_dir(&self) -> PathBuf {
        self.projects_dir().join(&self.project_slug)
    }

    fn project_partition_index_file(&self) -> PathBuf {
        self.project_partition_dir().join("README.md")
    }

    fn project_partition_channels_dir(&self) -> PathBuf {
        self.project_partition_dir().join("channels")
    }

    fn project_partition_channels_index_file(&self) -> PathBuf {
        self.project_partition_channels_dir().join("README.md")
    }

    fn project_partition_channel_file(&self) -> Option<PathBuf> {
        self.channel_slug.as_ref().map(|slug| {
            self.project_partition_channels_dir()
                .join(format!("{slug}.md"))
        })
    }

    fn project_partition_daily_dir(&self) -> PathBuf {
        self.project_partition_dir().join("daily")
    }

    fn project_partition_daily_index_file(&self) -> PathBuf {
        self.project_partition_daily_dir().join("README.md")
    }

    fn project_partition_daily_file(&self) -> PathBuf {
        self.project_partition_daily_dir()
            .join(format!("{}.md", self.today_slug))
    }

    fn project_partition_audit_dir(&self) -> PathBuf {
        self.project_partition_dir().join("audit")
    }

    fn project_partition_audit_index_file(&self) -> PathBuf {
        self.project_partition_audit_dir().join("README.md")
    }

    fn project_partition_cron_audit_dir(&self) -> PathBuf {
        self.project_partition_audit_dir().join("cron")
    }

    fn project_partition_cron_audit_file(&self, audit_day_slug: &str) -> PathBuf {
        self.project_partition_cron_audit_dir()
            .join(format!("{audit_day_slug}.md"))
    }

    fn channels_dir(&self) -> PathBuf {
        self.memory_dir().join("channels")
    }

    fn channel_file(&self) -> Option<PathBuf> {
        self.channel_slug
            .as_ref()
            .map(|slug| self.channels_dir().join(format!("{slug}.md")))
    }

    fn agents_dir(&self) -> PathBuf {
        self.memory_dir().join("agents")
    }

    fn agent_file(&self) -> Option<PathBuf> {
        self.agent_slug
            .as_ref()
            .map(|slug| self.agents_dir().join(format!("{slug}.md")))
    }

    fn topics_dir(&self) -> PathBuf {
        self.memory_dir().join("topics")
    }

    fn rules_file(&self) -> PathBuf {
        self.topics_dir().join("rules.md")
    }

    fn lessons_file(&self) -> PathBuf {
        self.topics_dir().join("lessons.md")
    }

    fn handoffs_dir(&self) -> PathBuf {
        self.memory_dir().join("handoffs")
    }

    fn archive_dir(&self) -> PathBuf {
        self.memory_dir().join("archive")
    }

    // --- Deep hierarchy paths ---

    fn daily_month_dir(&self) -> PathBuf {
        let (year_month, _day) = split_date_slug(&self.today_slug);
        self.daily_dir().join(year_month)
    }

    fn daily_day_dir(&self) -> PathBuf {
        let (year_month, day) = split_date_slug(&self.today_slug);
        self.daily_dir().join(year_month).join(day)
    }

    fn project_plans_dir(&self) -> PathBuf {
        self.project_partition_dir().join("plans")
    }

    fn project_decisions_dir(&self) -> PathBuf {
        self.project_partition_dir().join("decisions")
    }

    fn project_decisions_log(&self) -> PathBuf {
        self.project_decisions_dir().join("log.md")
    }

    fn project_status_dir(&self) -> PathBuf {
        self.project_partition_dir().join("status")
    }

    fn project_status_current(&self) -> PathBuf {
        self.project_status_dir().join("current.md")
    }

    fn project_reference_dir(&self) -> PathBuf {
        self.project_partition_dir().join("reference")
    }

    fn ops_dir(&self) -> PathBuf {
        self.memory_dir().join("ops")
    }

    fn ops_infra_dir(&self) -> PathBuf {
        self.ops_dir().join("infra")
    }

    fn ops_rules_dir(&self) -> PathBuf {
        self.ops_dir().join("rules")
    }

    fn ops_sns_dir(&self) -> PathBuf {
        self.ops_dir().join("sns")
    }

    fn channels_internal_dir(&self) -> PathBuf {
        self.channels_dir().join("internal")
    }

    fn channels_external_dir(&self) -> PathBuf {
        self.channels_dir().join("external")
    }

    fn bots_dir(&self) -> PathBuf {
        self.memory_dir().join("bots")
    }

    fn bounties_dir(&self) -> PathBuf {
        self.memory_dir().join("bounties")
    }

    fn bounties_active_dir(&self) -> PathBuf {
        self.bounties_dir().join("active")
    }

    fn bounties_prompts_dir(&self) -> PathBuf {
        self.bounties_dir().join("prompts")
    }

    fn bounties_archive_dir(&self) -> PathBuf {
        self.bounties_dir().join("archive")
    }

    fn research_dir(&self) -> PathBuf {
        self.memory_dir().join("research")
    }

    fn research_articles_dir(&self) -> PathBuf {
        self.research_dir().join("articles")
    }

    fn research_proposals_dir(&self) -> PathBuf {
        self.research_dir().join("proposals")
    }

    fn research_topics_dir(&self) -> PathBuf {
        self.research_dir().join("topics")
    }

    fn root_lessons_file(&self) -> PathBuf {
        self.memory_dir().join("lessons.md")
    }

    fn expected_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![
            self.memory_dir(),
            self.daily_dir(),
            self.projects_dir(),
            self.project_partition_dir(),
            self.project_partition_channels_dir(),
            self.project_partition_daily_dir(),
            self.project_partition_audit_dir(),
            self.project_partition_cron_audit_dir(),
            self.channels_dir(),
            self.agents_dir(),
            self.topics_dir(),
            self.handoffs_dir(),
            self.archive_dir(),
        ];
        if self.deep {
            dirs.extend([
                self.daily_month_dir(),
                self.daily_day_dir(),
                self.project_plans_dir(),
                self.project_decisions_dir(),
                self.project_status_dir(),
                self.project_reference_dir(),
                self.ops_dir(),
                self.ops_infra_dir(),
                self.ops_rules_dir(),
                self.ops_sns_dir(),
                self.channels_internal_dir(),
                self.channels_external_dir(),
                self.bots_dir(),
                self.bounties_dir(),
                self.bounties_active_dir(),
                self.bounties_prompts_dir(),
                self.bounties_archive_dir(),
                self.research_dir(),
                self.research_articles_dir(),
                self.research_proposals_dir(),
                self.research_topics_dir(),
            ]);
        }
        dirs
    }

    fn expected_files(&self) -> Vec<PathBuf> {
        let mut files = vec![
            self.memory_file(),
            self.memory_index_file(),
            self.scaffold_config_file(),
            self.daily_file(),
            self.project_file(),
            self.project_partition_index_file(),
            self.project_partition_channels_index_file(),
            self.project_partition_daily_index_file(),
            self.project_partition_daily_file(),
            self.project_partition_audit_index_file(),
            self.rules_file(),
            self.lessons_file(),
            self.handoffs_dir().join(".gitkeep"),
            self.archive_dir().join(".gitkeep"),
        ];
        if let Some(path) = self.channel_file() {
            files.push(path);
        }
        if let Some(path) = self.project_partition_channel_file() {
            files.push(path);
        }
        if let Some(path) = self.agent_file() {
            files.push(path);
        }
        if self.deep {
            files.extend([
                self.project_decisions_log(),
                self.project_status_current(),
                self.root_lessons_file(),
            ]);
            if self.daily_folder {
                files.extend([
                    self.daily_day_dir().join(format!("{}.md", self.project_slug)),
                    self.daily_day_dir().join("heartbeat.md"),
                    self.daily_day_dir().join("lessons.md"),
                    self.daily_day_dir().join("directives.md"),
                ]);
            }
        }
        files
    }
}

fn initialize_layout(layout: &MemoryLayout, force: bool) -> Result<MemoryInitReport> {
    for dir in layout.expected_dirs() {
        fs::create_dir_all(&dir)
            .with_context(|| format!("create memory directory {}", dir.display()))?;
    }

    let mut written_files = Vec::new();
    let mut skipped_files = Vec::new();
    for (path, contents) in scaffold_files(layout) {
        if write_scaffold_file(&path, &contents, force)? {
            written_files.push(path);
        } else {
            skipped_files.push(path);
        }
    }

    Ok(MemoryInitReport {
        written_files,
        skipped_files,
    })
}

fn inspect_layout(layout: &MemoryLayout) -> Result<MemoryStatusReport> {
    let memory_dir = layout.memory_dir();
    let markdown_file_count = count_markdown_files(&memory_dir)?;

    let mut missing_paths = Vec::new();
    for path in layout.expected_dirs() {
        if !path.is_dir() {
            missing_paths.push(path);
        }
    }
    for path in layout.expected_files() {
        if !path.exists() {
            missing_paths.push(path);
        }
    }

    Ok(MemoryStatusReport {
        layout: layout.clone(),
        memory_file_exists: layout.memory_file().is_file(),
        memory_dir_exists: memory_dir.is_dir(),
        markdown_file_count,
        missing_paths,
    })
}

fn tag_header(layout: &MemoryLayout, tags: &[&str]) -> String {
    if !layout.tags {
        return String::new();
    }
    let tag_list = tags.join(" ");
    format!("> 태그: {tag_list}\n\n")
}

fn split_date_slug(slug: &str) -> (&str, &str) {
    // "YYYY-MM-DD" -> ("YYYY-MM", "DD")
    let year_month = &slug[..7];
    let day = &slug[8..10];
    (year_month, day)
}

fn scaffold_files(layout: &MemoryLayout) -> Vec<(PathBuf, String)> {
    let mut files = vec![
        (layout.memory_file(), render_memory_md(layout)),
        (layout.memory_index_file(), render_memory_index(layout)),
        (
            layout.scaffold_config_file(),
            render_scaffold_config(layout),
        ),
        (layout.daily_file(), render_daily_file(layout)),
        (layout.project_file(), render_project_file(layout)),
        (
            layout.project_partition_index_file(),
            render_project_partition_index(layout),
        ),
        (
            layout.project_partition_channels_index_file(),
            render_project_partition_channels_index(layout),
        ),
        (
            layout.project_partition_daily_index_file(),
            render_project_partition_daily_index(layout),
        ),
        (
            layout.project_partition_daily_file(),
            render_project_partition_daily_file(layout),
        ),
        (
            layout.project_partition_audit_index_file(),
            render_project_partition_audit_index(layout),
        ),
        (layout.rules_file(), render_rules_file()),
        (layout.lessons_file(), render_lessons_file()),
        (
            layout.handoffs_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init\n"),
        ),
        (
            layout.archive_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init\n"),
        ),
    ];

    if let Some(path) = layout.channel_file() {
        files.push((path, render_channel_file(layout)));
    }
    if let Some(path) = layout.project_partition_channel_file() {
        files.push((path, render_project_partition_channel_file(layout)));
    }
    if let Some(path) = layout.agent_file() {
        files.push((path, render_agent_file(layout)));
    }

    if layout.deep {
        files.extend(deep_scaffold_files(layout));
    }

    files
}

fn deep_scaffold_files(layout: &MemoryLayout) -> Vec<(PathBuf, String)> {
    let mut files = vec![
        (
            layout.project_decisions_log(),
            format!(
                "{}# decisions log\n\n- Record architectural and workflow decisions here.\n- One entry per decision with date and rationale.\n",
                tag_header(layout, &["#decisions", &format!("#{}", layout.project_slug)]),
            ),
        ),
        (
            layout.project_status_current(),
            format!(
                "{}# current status\n\n- Active project: `{}`\n- Update this file with current priorities, blockers, and next steps.\n",
                tag_header(layout, &["#status", &format!("#{}", layout.project_slug)]),
                layout.project_slug,
            ),
        ),
        (
            layout.root_lessons_file(),
            format!(
                "{}# lessons\n\n- Root-level lessons promoted from project-scoped daily logs.\n- Keep one lesson per bullet so agents can scan quickly.\n",
                tag_header(layout, &["#lessons"]),
            ),
        ),
        (
            layout.project_plans_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.project_reference_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.ops_infra_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.ops_rules_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.ops_sns_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.channels_internal_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.channels_external_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.bots_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.bounties_active_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.bounties_prompts_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.bounties_archive_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.research_articles_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.research_proposals_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
        (
            layout.research_topics_dir().join(".gitkeep"),
            String::from("# tracked by clawhip memory init --hierarchy deep\n"),
        ),
    ];

    if layout.daily_folder {
        files.extend(daily_folder_files(layout));
    }

    files
}

fn daily_folder_files(layout: &MemoryLayout) -> Vec<(PathBuf, String)> {
    let day_dir = layout.daily_day_dir();
    vec![
        (
            day_dir.join(format!("{}.md", layout.project_slug)),
            format!(
                "{}# {} — {}\n\n## Summary\n\n- Per-project daily log.\n\n## Log\n\n- Scaffold created with `clawhip memory init`.\n",
                tag_header(layout, &["#daily", &format!("#{}", layout.project_slug)]),
                layout.today_slug,
                layout.project_slug,
            ),
        ),
        (
            day_dir.join("heartbeat.md"),
            format!(
                "{}# {} — heartbeat\n\n- Compressed heartbeat log for the day.\n",
                tag_header(layout, &["#daily", "#heartbeat"]),
                layout.today_slug,
            ),
        ),
        (
            day_dir.join("lessons.md"),
            format!(
                "{}# {} — lessons\n\n- Deduplicated learnings from the day.\n",
                tag_header(layout, &["#daily", "#lessons"]),
                layout.today_slug,
            ),
        ),
        (
            day_dir.join("directives.md"),
            format!(
                "{}# {} — directives\n\n- Owner directives for the day.\n",
                tag_header(layout, &["#daily", "#directives"]),
                layout.today_slug,
            ),
        ),
    ]
}

fn write_scaffold_file(path: &Path, contents: &str, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent directory {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("write scaffold file {}", path.display()))?;
    Ok(true)
}

fn render_memory_md(layout: &MemoryLayout) -> String {
    let mut quick_map = vec![
        format!(
            "- Project partition: `memory/projects/{}/README.md`",
            layout.project_slug
        ),
        format!(
            "- Today's canonical log: `memory/projects/{}/daily/{}.md`",
            layout.project_slug, layout.today_slug
        ),
        format!(
            "- Project shard pointer: `memory/projects/{}.md`",
            layout.project_slug
        ),
        format!("- Daily pointer: `memory/daily/{}.md`", layout.today_slug),
        String::from("- Durable rules: `memory/topics/rules.md`"),
        String::from("- Durable lessons: `memory/topics/lessons.md`"),
        String::from("- Full subtree guide: `memory/README.md`"),
    ];
    if let Some(channel) = &layout.channel_slug {
        quick_map.insert(
            2,
            format!(
                "- Channel partition: `memory/projects/{}/channels/{channel}.md`",
                layout.project_slug
            ),
        );
        quick_map.insert(
            3,
            format!("- Channel pointer: `memory/channels/{channel}.md`"),
        );
    }
    if let Some(agent) = &layout.agent_slug {
        quick_map.push(format!("- Agent profile: `memory/agents/{agent}.md`"));
    }

    let mut read_when = vec![
        format!(
            "- You need repo/project status -> read `memory/projects/{}/README.md`",
            layout.project_slug
        ),
        format!(
            "- You need latest execution context -> read `memory/projects/{}/daily/{}.md`",
            layout.project_slug, layout.today_slug
        ),
        String::from("- You are changing workflow policy -> read `memory/topics/rules.md`"),
    ];
    if let Some(channel) = &layout.channel_slug {
        read_when.insert(
            2,
            format!(
                "- You are acting in one channel/lane -> read `memory/projects/{}/channels/{channel}.md`",
                layout.project_slug
            ),
        );
    }

    format!(
        "# MEMORY.md — pointer/index layer

## Current beliefs

- Current priority: keep the hot memory layer small and push durable detail into `memory/`.
- Root memory is for summaries, pointers, and write obligations only.
- Detailed logs belong in project-scoped partitions.

## Quick file map

{}

## Read this when...

{}

## Write obligations

- Daily progress goes to `memory/projects/{}/daily/{}.md`.
- Project-specific detail goes to `memory/projects/{}/README.md`.
- Durable lessons get promoted into `memory/topics/lessons.md`.
- `MEMORY.md` only changes when the pointer map or current beliefs change.
",
        quick_map.join("\n"),
        read_when.join("\n"),
        layout.project_slug,
        layout.today_slug,
        layout.project_slug,
    )
}

fn render_memory_index(layout: &MemoryLayout) -> String {
    let mut file_map = vec![
        format!(
            "- `projects/{}/README.md` -> canonical project partition root",
            layout.project_slug
        ),
        format!(
            "- `projects/{}/daily/YYYY-MM-DD.md` -> canonical daily log",
            layout.project_slug
        ),
        String::from("- `topics/rules.md` -> durable operating rules"),
        String::from("- `topics/lessons.md` -> reusable lessons"),
        String::from("- `handoffs/YYYY-MM-DD-<slug>.md` -> bounded handoffs"),
        String::from("- `archive/YYYY-MM/` -> cold history"),
    ];
    if let Some(channel) = &layout.channel_slug {
        file_map.insert(
            1,
            format!("- `channels/{channel}.md` -> compatibility pointer"),
        );
        file_map.insert(
            2,
            format!(
                "- `projects/{}/channels/{channel}.md` -> canonical channel partition",
                layout.project_slug
            ),
        );
    } else {
        file_map.insert(
            1,
            String::from("- `channels/<channel>.md` -> compatibility pointer"),
        );
        file_map.insert(
            2,
            format!(
                "- `projects/{}/channels/<channel>.md` -> canonical channel partition",
                layout.project_slug
            ),
        );
    }
    if let Some(agent) = &layout.agent_slug {
        file_map.insert(
            3,
            format!("- `agents/{agent}.md` -> one agent/operator profile"),
        );
    } else {
        file_map.insert(
            3,
            String::from("- `agents/<agent>.md` -> one agent/operator profile"),
        );
    }
    file_map.insert(
        4,
        format!(
            "- `projects/{}/audit/cron/YYYY-MM-DD.md` -> cron audit log",
            layout.project_slug
        ),
    );

    format!(
        "# memory/README.md — retrieval guide

## File map

{}

## Read by situation

- Need latest execution context -> latest file in `projects/{}/daily/`
- Need canonical project state -> `projects/{}/README.md`
- Need policy or norms -> `topics/rules.md`

## Naming rules

- Use stable slugs for channels, projects, and agents.
- Keep flat `memory/daily/*.md` and `memory/channels/*.md` files as compatibility pointers.
- Keep `MEMORY.md` short; move durable detail into leaf files.
- Archive inactive history instead of bloating the hot path.
",
        file_map.join("\n"),
        layout.project_slug,
        layout.project_slug,
    )
}

fn render_scaffold_config(layout: &MemoryLayout) -> String {
    let mut legacy_entries = vec![
        format!("project_pointer = \"projects/{}.md\"", layout.project_slug),
        format!("daily_pointer = \"daily/{}.md\"", layout.today_slug),
    ];
    if let Some(channel) = &layout.channel_slug {
        legacy_entries.push(format!("channel_pointer = \"channels/{channel}.md\""));
    }

    format!(
        "version = 1

[project]
slug = \"{project}\"
root = \"projects/{project}\"

[partitions]
project = \"projects/{project}/README.md\"
daily_dir = \"projects/{project}/daily\"
channel_dir = \"projects/{project}/channels\"
cron_audit_dir = \"projects/{project}/audit/cron\"

[legacy]
{legacy}
",
        project = layout.project_slug,
        legacy = legacy_entries.join("\n"),
    )
}

fn render_daily_file(layout: &MemoryLayout) -> String {
    format!(
        "# {}

## Role

- Compatibility pointer for older readers that still open `memory/daily/*.md`.
- Canonical daily partition: `memory/projects/{}/daily/{}.md`

## Keep here

- a short redirect only
- no long-form execution detail
",
        layout.today_slug, layout.project_slug, layout.today_slug
    )
}

fn render_project_file(layout: &MemoryLayout) -> String {
    format!(
        "# {}

## Role

- Compatibility pointer for older readers that still open `memory/projects/{}.md`.
- Canonical project partition: `memory/projects/{}/README.md`

## Keep here

- a short redirect only
- no long-form project detail
",
        layout.project_slug, layout.project_slug, layout.project_slug
    )
}

fn render_channel_file(layout: &MemoryLayout) -> String {
    let channel = layout
        .channel_slug
        .as_deref()
        .expect("channel file only rendered when channel slug exists");
    format!(
        "# {}

## Role

- Compatibility pointer for older readers that still open `memory/channels/{channel}.md`.
- Canonical channel partition: `memory/projects/{}/channels/{channel}.md`

## Related shards

- project state -> `memory/projects/{}/README.md`
- daily execution log -> `memory/projects/{}/daily/{}.md`
- durable rules -> `memory/topics/rules.md`
",
        channel, layout.project_slug, layout.project_slug, layout.project_slug, layout.today_slug
    )
}

fn render_project_partition_index(layout: &MemoryLayout) -> String {
    format!(
        "# {}

## Current state

- Canonical project partition for repo-specific status.
- Nested daily/channel/audit shards for this project live beside this file.

## Canonical children

- daily logs -> `daily/`
- channel lanes -> `channels/`
- cron audit notes -> `audit/cron/`

## Keep here

- project status
- active priorities
- blockers and follow-ups
- links to handoffs and decisions
",
        layout.project_slug
    )
}

fn render_project_partition_channels_index(layout: &MemoryLayout) -> String {
    format!(
        "# channels for {}

- Canonical per-channel partitions for this project live here.
- Keep each lane in its own file.
",
        layout.project_slug
    )
}

fn render_project_partition_daily_index(layout: &MemoryLayout) -> String {
    format!(
        "# daily for {}

- Canonical daily execution logs for this project live here.
- Keep one file per UTC day using `YYYY-MM-DD.md`.
",
        layout.project_slug
    )
}

fn render_project_partition_daily_file(layout: &MemoryLayout) -> String {
    let mut lines = vec![
        format!("# {}", layout.today_slug),
        String::new(),
        "## Summary".into(),
        String::new(),
        format!("- Active project: `{}`", layout.project_slug),
        "- Canonical daily log inside the project partition.".into(),
    ];
    if let Some(channel) = &layout.channel_slug {
        lines.push(format!("- Active channel lane: `../channels/{channel}.md`"));
    }
    if let Some(agent) = &layout.agent_slug {
        lines.push(format!(
            "- Active agent profile: `../../../agents/{agent}.md`"
        ));
    }
    lines.extend([
        String::new(),
        "## Log".into(),
        String::new(),
        "- Scaffold created with `clawhip memory init`.".into(),
    ]);
    lines.join("\n") + "\n"
}

fn render_project_partition_channel_file(layout: &MemoryLayout) -> String {
    let channel = layout
        .channel_slug
        .as_deref()
        .expect("project channel file only rendered when channel slug exists");
    format!(
        "# {}

## Role

- Canonical memory for one channel or workflow lane inside the project partition.
- Keep local context, commitments, and lane-specific follow-ups here.

## Related shards

- project state -> `../README.md`
- daily execution log -> `../daily/{}.md`
- durable rules -> `../../../topics/rules.md`
",
        channel, layout.today_slug
    )
}

fn render_project_partition_audit_index(layout: &MemoryLayout) -> String {
    format!(
        "# audit for {}

- Cron-driven scaffold audits append markdown notes under `cron/YYYY-MM-DD.md`.
- Use this directory for machine-generated integrity checks and short operator follow-ups.
",
        layout.project_slug
    )
}

fn render_agent_file(layout: &MemoryLayout) -> String {
    let agent = layout
        .agent_slug
        .as_deref()
        .expect("agent file only rendered when agent slug exists");
    format!(
        "# {}

## Role

- Canonical memory for one agent or operator profile.
- Keep preferences, handoff expectations, and recurring operating notes here.

## Related shards

- project state -> `memory/projects/{}/README.md`
- daily execution log -> `memory/projects/{}/daily/{}.md`
- durable lessons -> `memory/topics/lessons.md`
",
        agent, layout.project_slug, layout.project_slug, layout.today_slug
    )
}

fn render_rules_file() -> String {
    String::from(
        "# rules

- Root `MEMORY.md` stays short and skimmable.
- Durable workflow rules live here, not in the daily log.
- Refactor noisy memory into dedicated shards instead of growing one hot file.
",
    )
}

fn render_lessons_file() -> String {
    String::from(
        "# lessons

- Promote reusable lessons here after they become stable.
- Keep one lesson per bullet or subsection so agents can scan quickly.
",
    )
}

fn append_cron_audit_report(
    report: &MemoryStatusReport,
    report_path: &Path,
    generated_at: OffsetDateTime,
) -> Result<()> {
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create audit directory {}", parent.display()))?;
    }

    let header = format!(
        "# cron audits for {} on {:04}-{:02}-{:02}\n\n",
        report.layout.project_slug,
        generated_at.year(),
        u8::from(generated_at.month()),
        generated_at.day()
    );
    let status_label = if report.missing_paths.is_empty() {
        "ready"
    } else {
        "missing-paths"
    };
    let mut entry = format!(
        "## {}\n\n- Status: `{status_label}`\n- Markdown files under `memory/`: {}\n- Project partition: `memory/projects/{}/README.md`\n- Daily partition: `memory/projects/{}/daily/{}.md`\n",
        audit_timestamp(generated_at),
        report.markdown_file_count,
        report.layout.project_slug,
        report.layout.project_slug,
        report.layout.today_slug
    );
    if let Some(channel) = &report.layout.channel_slug {
        entry.push_str(&format!(
            "- Channel partition: `memory/projects/{}/channels/{channel}.md`\n",
            report.layout.project_slug
        ));
    }
    if report.missing_paths.is_empty() {
        entry.push_str("- Missing paths: none\n");
    } else {
        entry.push_str("- Missing paths:\n");
        for path in &report.missing_paths {
            entry.push_str(&format!(
                "  - `{}`\n",
                display_relative(&report.layout.root, path)
            ));
        }
    }
    entry.push('\n');

    let existing = if report_path.exists() {
        fs::read_to_string(report_path)
            .with_context(|| format!("read existing audit report {}", report_path.display()))?
    } else {
        String::new()
    };
    let contents = if existing.is_empty() {
        format!("{header}{entry}")
    } else {
        format!("{existing}{entry}")
    };
    fs::write(report_path, contents)
        .with_context(|| format!("write audit report {}", report_path.display()))?;
    Ok(())
}

fn count_markdown_files(root: &Path) -> Result<usize> {
    if !root.exists() {
        return Ok(0);
    }

    let mut count = 0;
    for entry in fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            count += count_markdown_files(&path)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            count += 1;
        }
    }
    Ok(count)
}

fn slugify(input: &str) -> Result<String> {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in input.trim().chars() {
        let normalized = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            ' ' | '_' | '-' | '/' | '.' => Some('-'),
            _ => None,
        };

        let Some(ch) = normalized else {
            continue;
        };

        if ch == '-' {
            if slug.is_empty() || last_was_dash {
                continue;
            }
            last_was_dash = true;
            slug.push(ch);
        } else {
            last_was_dash = false;
            slug.push(ch);
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        Err(anyhow!("unable to derive a stable slug from '{input}'").into())
    } else {
        Ok(slug)
    }
}

fn normalize_date_slug(date: Option<String>) -> Result<String> {
    match date {
        Some(date) => {
            let trimmed = date.trim();
            validate_date_slug(trimmed)?;
            Ok(trimmed.to_string())
        }
        None => Ok(today_slug()),
    }
}

pub(crate) fn validate_date_slug(value: &str) -> Result<()> {
    if is_valid_date_slug(value) {
        Ok(())
    } else {
        Err(anyhow!("date must use YYYY-MM-DD format").into())
    }
}

fn is_valid_date_slug(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn today_slug() -> String {
    let date = time::OffsetDateTime::now_utc().date();
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn audit_timestamp(timestamp: OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        timestamp.year(),
        u8::from(timestamp.month()),
        timestamp.day(),
        timestamp.hour(),
        timestamp.minute(),
        timestamp.second()
    )
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Date, Month, PrimitiveDateTime, Time};

    #[test]
    fn init_creates_memory_scaffold() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: Some("alerts".into()),
            agent_slug: Some("codex".into()),
            today_slug: "2026-03-10".into(),
            deep: false,
            daily_folder: false,
            tags: false,
        };

        let report = initialize_layout(&layout, false).expect("initialize layout");

        assert!(report.written_files.contains(&layout.memory_file()));
        assert!(layout.memory_file().is_file());
        assert!(layout.memory_index_file().is_file());
        assert!(layout.scaffold_config_file().is_file());
        assert!(layout.project_file().is_file());
        assert!(layout.daily_file().is_file());
        assert!(layout.project_partition_index_file().is_file());
        assert!(layout.project_partition_daily_index_file().is_file());
        assert!(layout.project_partition_daily_file().is_file());
        assert!(layout.project_partition_audit_index_file().is_file());
        assert!(layout.rules_file().is_file());
        assert!(layout.lessons_file().is_file());
        assert!(layout.channel_file().expect("channel").is_file());
        assert!(
            layout
                .project_partition_channel_file()
                .expect("channel")
                .is_file()
        );
        assert!(layout.agent_file().expect("agent").is_file());
        assert!(layout.handoffs_dir().join(".gitkeep").is_file());
        assert!(layout.archive_dir().join(".gitkeep").is_file());

        let memory_md = fs::read_to_string(layout.memory_file()).expect("read MEMORY.md");
        assert!(memory_md.contains("memory/projects/clawhip/README.md"));
        assert!(memory_md.contains("memory/channels/alerts.md"));
        assert!(memory_md.contains("memory/agents/codex.md"));

        let scaffold = fs::read_to_string(layout.scaffold_config_file()).expect("read scaffold");
        assert!(scaffold.contains("cron_audit_dir = \"projects/clawhip/audit/cron\""));
    }

    #[test]
    fn init_keeps_existing_files_without_force() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-03-10".into(),
            deep: false,
            daily_folder: false,
            tags: false,
        };

        fs::create_dir_all(layout.memory_dir()).expect("create memory dir");
        fs::write(layout.memory_file(), "custom memory").expect("write existing memory file");

        let report = initialize_layout(&layout, false).expect("initialize layout");

        assert!(report.skipped_files.contains(&layout.memory_file()));
        assert_eq!(
            fs::read_to_string(layout.memory_file()).expect("read MEMORY.md"),
            "custom memory"
        );
    }

    #[test]
    fn inspect_reports_missing_recommended_paths() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: Some("alerts".into()),
            agent_slug: None,
            today_slug: "2026-03-10".into(),
            deep: false,
            daily_folder: false,
            tags: false,
        };

        fs::create_dir_all(layout.memory_dir()).expect("create memory dir");

        let report = inspect_layout(&layout).expect("inspect layout");

        assert!(report.memory_dir_exists);
        assert!(!report.memory_file_exists);
        assert!(report.missing_paths.contains(&layout.memory_file()));
        assert!(report.missing_paths.contains(&layout.project_file()));
        assert!(
            report
                .missing_paths
                .contains(&layout.project_partition_index_file())
        );
        assert!(
            report
                .missing_paths
                .contains(&layout.channel_file().expect("channel"))
        );
        assert!(
            report
                .missing_paths
                .contains(&layout.project_partition_channel_file().expect("channel"))
        );
    }

    #[test]
    fn slugify_normalizes_common_inputs() {
        assert_eq!(
            slugify("Clawhip Workspace").expect("slug"),
            "clawhip-workspace"
        );
        assert_eq!(
            slugify("issue_73/runtime").expect("slug"),
            "issue-73-runtime"
        );
    }

    #[test]
    fn slugify_rejects_empty_results() {
        let error = slugify("!!!").expect_err("invalid slug should fail");
        assert!(error.to_string().contains("stable slug"));
    }

    #[test]
    fn normalize_date_slug_accepts_iso_dates() {
        assert_eq!(
            normalize_date_slug(Some("2026-03-10".into())).expect("date slug"),
            "2026-03-10"
        );
    }

    #[test]
    fn normalize_date_slug_rejects_non_iso_dates() {
        let error = normalize_date_slug(Some("03/10/2026".into())).expect_err("invalid date");
        assert!(error.to_string().contains("YYYY-MM-DD"));
    }

    #[test]
    fn cron_audit_writes_report_into_project_partition() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: Some("alerts".into()),
            agent_slug: Some("codex".into()),
            today_slug: "2026-03-10".into(),
            deep: false,
            daily_folder: false,
            tags: false,
        };
        initialize_layout(&layout, false).expect("initialize layout");

        let generated_at = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::March, 31).expect("date"),
            Time::from_hms(9, 15, 0).expect("time"),
        )
        .assume_utc();
        let audit = run_cron_audit(
            tempdir.path().to_path_buf(),
            Some("clawhip".into()),
            Some("alerts".into()),
            Some("codex".into()),
            Some("2026-03-10".into()),
            generated_at,
        )
        .expect("run audit");

        assert!(audit.missing_paths.is_empty());
        assert!(audit.summary.contains("memory audit ok"));
        assert_eq!(
            audit.report_path,
            layout.project_partition_cron_audit_file("2026-03-31")
        );

        let report = fs::read_to_string(&audit.report_path).expect("read audit report");
        assert!(report.contains("Status: `ready`"));
        assert!(report.contains("memory/projects/clawhip/channels/alerts.md"));
    }

    #[test]
    fn deep_hierarchy_creates_full_tree() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-04-06".into(),
            deep: true,
            daily_folder: true,
            tags: true,
        };

        let report = initialize_layout(&layout, false).expect("initialize layout");

        assert!(layout.project_decisions_log().is_file());
        assert!(layout.project_status_current().is_file());
        assert!(layout.root_lessons_file().is_file());
        assert!(layout.ops_infra_dir().is_dir());
        assert!(layout.ops_rules_dir().is_dir());
        assert!(layout.ops_sns_dir().is_dir());
        assert!(layout.channels_internal_dir().is_dir());
        assert!(layout.channels_external_dir().is_dir());
        assert!(layout.bots_dir().is_dir());
        assert!(layout.bounties_active_dir().is_dir());
        assert!(layout.research_articles_dir().is_dir());
        assert!(layout.research_proposals_dir().is_dir());
        assert!(layout.research_topics_dir().is_dir());

        // daily folder structure
        assert!(layout.daily_day_dir().is_dir());
        assert!(layout.daily_day_dir().join("clawhip.md").is_file());
        assert!(layout.daily_day_dir().join("heartbeat.md").is_file());
        assert!(layout.daily_day_dir().join("lessons.md").is_file());
        assert!(layout.daily_day_dir().join("directives.md").is_file());

        // tag headers present
        let decisions = fs::read_to_string(layout.project_decisions_log()).expect("read");
        assert!(decisions.contains("태그:"));
        assert!(decisions.contains("#decisions"));

        let heartbeat = fs::read_to_string(layout.daily_day_dir().join("heartbeat.md")).expect("read");
        assert!(heartbeat.contains("태그:"));
        assert!(heartbeat.contains("#heartbeat"));

        assert!(!report.written_files.is_empty());
    }

    #[test]
    fn tags_flag_adds_headers_to_deep_files() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "test".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-04-06".into(),
            deep: true,
            daily_folder: false,
            tags: true,
        };

        initialize_layout(&layout, false).expect("initialize layout");

        let status = fs::read_to_string(layout.project_status_current()).expect("read");
        assert!(status.starts_with("> 태그:"));
        assert!(status.contains("#status"));
    }

    #[test]
    fn tags_flag_off_omits_headers() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "test".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-04-06".into(),
            deep: true,
            daily_folder: false,
            tags: false,
        };

        initialize_layout(&layout, false).expect("initialize layout");

        let status = fs::read_to_string(layout.project_status_current()).expect("read");
        assert!(!status.contains("태그:"));
    }

    #[test]
    fn audit_detects_missing_status_and_decisions() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-04-06".into(),
            deep: false,
            daily_folder: false,
            tags: false,
        };

        // Create just the flat scaffold (no deep files)
        initialize_layout(&layout, false).expect("initialize layout");

        let issues = run_audit_checks(&layout).expect("audit");

        let has_status_issue = issues.iter().any(|i| i.contains("status/current.md"));
        let has_decisions_issue = issues.iter().any(|i| i.contains("decisions/log.md"));
        assert!(has_status_issue, "should flag missing status/current.md");
        assert!(has_decisions_issue, "should flag missing decisions/log.md");
    }

    #[test]
    fn audit_fix_creates_missing_files() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-04-06".into(),
            deep: false,
            daily_folder: false,
            tags: false,
        };

        initialize_layout(&layout, false).expect("initialize layout");

        let fixed = apply_audit_fixes(&layout).expect("fix");
        assert!(fixed >= 2);
        assert!(layout.project_status_current().is_file());
        assert!(layout.project_decisions_log().is_file());
    }

    #[test]
    fn rotate_creates_daily_folder() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let layout = MemoryLayout {
            root: tempdir.path().to_path_buf(),
            project_slug: "clawhip".into(),
            channel_slug: None,
            agent_slug: None,
            today_slug: "2026-04-06".into(),
            deep: true,
            daily_folder: true,
            tags: true,
        };

        let day_dir = layout.daily_day_dir();
        fs::create_dir_all(&day_dir).expect("create day dir");

        let files = daily_folder_files(&layout);
        for (path, contents) in &files {
            write_scaffold_file(path, contents, false).expect("write");
        }

        assert!(day_dir.join("clawhip.md").is_file());
        assert!(day_dir.join("heartbeat.md").is_file());
        assert!(day_dir.join("lessons.md").is_file());
        assert!(day_dir.join("directives.md").is_file());
    }

    #[test]
    fn split_date_slug_parses_correctly() {
        let (year_month, day) = split_date_slug("2026-04-06");
        assert_eq!(year_month, "2026-04");
        assert_eq!(day, "06");
    }
}
