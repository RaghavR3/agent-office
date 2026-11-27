//! The skills and slash commands Claude Code offers in a folder: the project's, yours, your
//! enabled plugins', and the built-in commands. Read-only: just names and one-line descriptions.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Skill {
    /// What you type after the slash.
    pub name: String,
    pub description: String,
    /// project | you | plugin | built-in
    pub source: String,
}

const BUILT_IN: [(&str, &str); 9] = [
    ("clear", "Start fresh: clear the conversation"),
    ("compact", "Shrink the conversation to free up room"),
    ("context", "See what's filling the context window"),
    ("mcp", "Manage MCP server connections"),
    ("rename", "Name this session"),
    ("model", "Change the model"),
    ("review", "Review a pull request"),
    ("init", "Write a CLAUDE.md for this project"),
    ("memory", "Edit Claude's memory files"),
];

/// `name` and `description` from a markdown file's front matter, and its first line of text.
fn read_meta(text: &str) -> (Option<String>, Option<String>, String) {
    let mut name = None;
    let mut description = None;
    let mut body = text;
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            body = &rest[end + 4..];
            let lines: Vec<&str> = front.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let Some((k, v)) = line.split_once(':') else { continue };
                let mut v = v.trim().trim_matches(['"', '\'']).to_string();
                // A folded description continues on the indented lines below.
                if v == ">" || v == "|" || v == ">-" || v == "|-" {
                    v = lines[i + 1..].iter().take_while(|l| l.starts_with(' ')).map(|l| l.trim()).collect::<Vec<_>>().join(" ");
                }
                match k.trim() {
                    "name" if !v.is_empty() => name = Some(v),
                    "description" if !v.is_empty() => description = Some(v),
                    _ => {}
                }
            }
        }
    }
    let first = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").trim_start_matches('#').trim().to_string();
    (name, description, first)
}

fn add(out: &mut Vec<Skill>, name: String, description: String, source: &str) {
    if name.is_empty() || out.iter().any(|s| s.name == name) {
        return;
    }
    out.push(Skill { name, description: crate::brief::clip(&description, 140), source: source.into() });
}

/// Skills are folders with a SKILL.md: `<dir>/<skill>/SKILL.md`.
fn skills_in(dir: &Path, prefix: &str, source: &str, out: &mut Vec<Skill>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join("SKILL.md").is_file()).collect();
    found.sort();
    for p in found {
        let Ok(text) = std::fs::read_to_string(p.join("SKILL.md")) else { continue };
        let (name, description, first) = read_meta(&text);
        let name = name.unwrap_or_else(|| p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string());
        add(out, format!("{prefix}{name}"), description.unwrap_or(first), source);
    }
}

/// Commands are markdown files, `<dir>/<cmd>.md`, or `<dir>/<group>/<cmd>.md` as `group:cmd`.
fn commands_in(dir: &Path, prefix: &str, source: &str, out: &mut Vec<Skill>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut found: Vec<(String, PathBuf)> = vec![];
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let group = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
            for inner in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                let q = inner.path();
                if q.extension().is_some_and(|x| x == "md") {
                    found.push((format!("{group}:{}", q.file_stem().and_then(|s| s.to_str()).unwrap_or_default()), q));
                }
            }
        } else if p.extension().is_some_and(|x| x == "md") {
            found.push((p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string(), p));
        }
    }
    found.sort();
    for (name, p) in found {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let (_, description, first) = read_meta(&text);
        add(out, format!("{prefix}{name}"), description.unwrap_or(first), source);
    }
}

/// Folders a project's own skills can live in: the folder itself and the repo it's in.
fn project_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = vec![cwd.to_path_buf()];
    if let Some(git) = cwd.ancestors().find(|a| a.join(".git").exists()) {
        if git != cwd {
            roots.push(git.to_path_buf());
        }
    }
    roots
}

pub fn list(cwd: Option<&Path>) -> Vec<Skill> {
    let home = dirs::home_dir().unwrap_or_default();
    let claude = home.join(".claude");
    let mut out = vec![];
    for root in cwd.map(project_roots).unwrap_or_default() {
        skills_in(&root.join(".claude/skills"), "", "project", &mut out);
        commands_in(&root.join(".claude/commands"), "", "project", &mut out);
    }
    skills_in(&claude.join("skills"), "", "you", &mut out);
    commands_in(&claude.join("commands"), "", "you", &mut out);

    // Enabled plugins, each skill named `<plugin>:<skill>`.
    let read = |p: PathBuf| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(Value::Null);
    let settings = read(claude.join("settings.json"));
    let installed = read(claude.join("plugins/installed_plugins.json"));
    let mut enabled: Vec<&String> = settings["enabledPlugins"].as_object().into_iter().flatten().filter(|(_, on)| on == &&Value::Bool(true)).map(|(k, _)| k).collect();
    enabled.sort();
    for key in enabled {
        let plugin = key.split('@').next().unwrap_or(key);
        let Some(path) = installed["plugins"][key.as_str()].as_array().and_then(|a| a.first()).and_then(|e| e["installPath"].as_str()) else { continue };
        let base = PathBuf::from(path);
        skills_in(&base.join("skills"), &format!("{plugin}:"), "plugin", &mut out);
        commands_in(&base.join("commands"), &format!("{plugin}:"), "plugin", &mut out);
    }
    for (name, description) in BUILT_IN {
        add(&mut out, name.into(), description.into(), "built-in");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_skill_front_matter() {
        let (n, d, _) = read_meta("---\nname: draft-prd\ndescription: Use when drafting a PRD\n---\n\n# Draft PRD\n");
        assert_eq!((n.as_deref(), d.as_deref()), (Some("draft-prd"), Some("Use when drafting a PRD")));
        let (_, d, _) = read_meta("---\ndescription: >\n  Folded over\n  two lines\n---\nBody");
        assert_eq!(d.as_deref(), Some("Folded over two lines"));
        let (n, d, first) = read_meta("# Ship it\nDeploys the app");
        assert_eq!((n, d, first.as_str()), (None, None, "Ship it"));
    }

    #[test]
    fn finds_project_skills_and_commands() {
        let dir = std::env::temp_dir().join(format!("ao-skills-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join(".claude/skills/release")).unwrap();
        std::fs::write(dir.join(".claude/skills/release/SKILL.md"), "---\nname: release\ndescription: Cut a release\n---\n").unwrap();
        std::fs::create_dir_all(dir.join(".claude/commands/db")).unwrap();
        std::fs::write(dir.join(".claude/commands/db/migrate.md"), "Run the migrations").unwrap();
        let list = list(Some(&dir));
        assert!(list.iter().any(|s| s.name == "release" && s.source == "project" && s.description == "Cut a release"));
        assert!(list.iter().any(|s| s.name == "db:migrate" && s.description == "Run the migrations"));
        assert!(list.iter().any(|s| s.name == "clear" && s.source == "built-in"));
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod live_check {
    /// `cargo test live_skills -- --ignored --nocapture` lists what the picker would offer here.
    #[test]
    #[ignore]
    fn live_skills() {
        let cwd = std::env::var("AO_CWD").ok();
        let list = super::list(cwd.as_deref().map(std::path::Path::new));
        let by = |s: &str| list.iter().filter(|x| x.source == s).count();
        println!("{} total: project {}, you {}, plugin {}, built-in {}", list.len(), by("project"), by("you"), by("plugin"), by("built-in"));
        for s in list.iter().filter(|s| s.source == "plugin").take(4) {
            println!("  /{} — {}", s.name, s.description);
        }
    }
}
