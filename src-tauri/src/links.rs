//! The links worth clicking from an agent's session: what it reported in its brief first, then
//! what it put in its end-of-turn messages, then things it clearly created. Links it only passed
//! by while working go to "more".

use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LinkItem {
    /// PR, Issue, Notion, Confluence, Linear, Doc, Sheet, Slides, Figma, Artifact, Preview, HubSpot, Link
    pub kind: String,
    pub label: String,
    pub url: String,
}

#[derive(Serialize, Default, Debug)]
pub struct Links {
    pub top: Vec<LinkItem>,
    pub more: Vec<LinkItem>,
}

/// Web links, and plain-http previews running on this Mac.
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https://[^\s)\]>"'`|,]+|http://(?:localhost|127\.0\.0\.1|0\.0\.0\.0)\b[^\s)\]>"'`|,]*"#).unwrap()
});
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]\n]{2,80})\]\((https://[^)\s]+|http://(?:localhost|127\.0\.0\.1|0\.0\.0\.0)\b[^)\s]*)\)").unwrap()
});
static GH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://github\.com/[^/]+/([^/]+)/(pull|issues)/(\d+)").unwrap());
static LINEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://linear\.app/[^/]+/issue/([A-Za-z]+-\d+)(?:/([^/?#]+))?").unwrap());
static HEX_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-?[0-9a-f]{32}$").unwrap());

fn words(slug: &str, max_words: usize) -> String {
    let s = slug.replace(['-', '+', '_'], " ").replace("%20", " ");
    let w: Vec<&str> = s.split_whitespace().collect();
    let mut out = w.iter().take(max_words).copied().collect::<Vec<_>>().join(" ");
    if w.len() > max_words {
        out.push('…');
    }
    let mut c = out.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn last_segment(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

/// Names a link by what it points to. `strict` keeps only links that are clearly work products,
/// for text the agent didn't write itself (tool output is full of incidental URLs).
pub fn classify(url: &str, strict: bool) -> Option<LinkItem> {
    let item = |kind: &str, label: String| Some(LinkItem { kind: kind.into(), label, url: url.into() });
    // A dev server on this Mac is always worth a click: it's the thing being built.
    if let Some(rest) = url.strip_prefix("http://") {
        let label = rest.replacen("0.0.0.0", "localhost", 1).replacen("127.0.0.1", "localhost", 1);
        return item("Preview", label.trim_end_matches('/').to_string());
    }
    let host = url.trim_start_matches("https://").split('/').next().unwrap_or("").trim_start_matches("www.");
    if let Some(c) = GH.captures(url) {
        let (kind, word) = if &c[2] == "pull" { ("PR", "PR") } else { ("Issue", "Issue") };
        return item(kind, format!("{word} #{} in {}", &c[3], &c[1]));
    }
    if let Some(c) = LINEAR.captures(url) {
        let title = c.get(2).map(|m| words(m.as_str(), 5)).unwrap_or_default();
        return item("Linear", if title.is_empty() { c[1].to_string() } else { format!("{} {}", &c[1], title) });
    }
    if host.ends_with("notion.so") || host.ends_with("notion.site") {
        let t = words(&HEX_TAIL.replace(last_segment(url), ""), 7);
        return item("Notion", if t.is_empty() { "Notion page".into() } else { t });
    }
    if host.ends_with("atlassian.net") && url.contains("/wiki/") {
        let t = words(last_segment(url), 7);
        let numeric = t.chars().all(|c| c.is_ascii_digit());
        return item("Confluence", if t.is_empty() || numeric { "Confluence page".into() } else { t });
    }
    if host == "docs.google.com" {
        let kind = if url.contains("/spreadsheets/") { "Sheet" } else if url.contains("/presentation/") { "Slides" } else { "Doc" };
        return item(kind, format!("Google {}", match kind { "Sheet" => "Sheet", "Slides" => "Slides", _ => "Doc" }));
    }
    if host.ends_with("figma.com") {
        let t = words(last_segment(url), 6);
        return item("Figma", if t.is_empty() { "Figma file".into() } else { t });
    }
    if host == "claude.ai" && url.contains("artifact") {
        return item("Artifact", "Artifact".into());
    }
    if host.ends_with("vercel.app") || host.ends_with("pages.dev") || host.ends_with("netlify.app") {
        return item("Preview", host.into());
    }
    if host.ends_with("hubspot.com") && host.starts_with("app") {
        return item("HubSpot", "HubSpot record".into());
    }
    if strict {
        return None;
    }
    item("Link", host.into())
}

struct Found {
    item: LinkItem,
    score: u32,
    order: usize,
}

fn text_of(content: &Value) -> Vec<String> {
    match content {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .flat_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(|t| vec![t.to_string()]).unwrap_or_default(),
                Some("tool_result") => text_of(&b["content"]),
                _ => vec![],
            })
            .collect(),
        _ => vec![],
    }
}

pub fn read_links(path: &Path) -> Result<Links> {
    Ok(parse_links(&crate::transcript::read_tail(path)?))
}

fn parse_links(raw: &str) -> Links {
    let mut found: HashMap<String, Found> = HashMap::new();
    let mut order = 0usize;
    let mut add = |url: &str, label: Option<&str>, score: u32, strict: bool| {
        let url = url.trim_end_matches(['.', ';', ':', '*', '_']);
        let Some(mut item) = classify(url, strict) else { return };
        // The agent's own words for a link beat anything guessed from the URL.
        if let Some(l) = label.map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("http")) {
            item.label = l.to_string();
        }
        order += 1;
        let e = found.entry(item.url.clone()).or_insert(Found { item: item.clone(), score: 0, order });
        if score >= e.score {
            if label.is_some() || e.score == 0 {
                e.item = item;
            }
            e.score = score;
        }
        e.order = order;
    };

    for line in raw.lines() {
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if d["isSidechain"] == Value::Bool(true) {
            continue;
        }
        match d["type"].as_str() {
            Some("assistant") => {
                let final_turn = d["message"]["stop_reason"] == "end_turn";
                for text in text_of(&d["message"]["content"]) {
                    if let Some(body) = crate::brief::fenced(&text, "brief") {
                        if let Ok(b) = serde_json::from_str::<crate::brief::Brief>(&body) {
                            for l in &b.links {
                                add(&l.url, Some(&l.label), 100, false);
                            }
                        }
                    }
                    let base = if final_turn { 50 } else { 10 };
                    for c in MD_LINK.captures_iter(&text) {
                        add(&c[2], Some(&c[1]), base + 5, false);
                    }
                    // End-of-turn messages are the agent reporting back, so any link counts there.
                    for m in URL.find_iter(&text) {
                        add(m.as_str(), None, base, !final_turn);
                    }
                }
            }
            // Tool output: only clear work products, like the PR URL `gh pr create` prints.
            Some("user") => {
                for text in text_of(&d["message"]["content"]) {
                    for m in URL.find_iter(&text) {
                        add(m.as_str(), None, 20, true);
                    }
                }
            }
            _ => {}
        }
    }

    let mut all: Vec<Found> = found.into_values().filter(|f| f.score >= 20).collect();
    all.sort_by(|a, b| b.score.cmp(&a.score).then(b.order.cmp(&a.order)));
    // Keep the top list varied: at most two of any kind, and only things the agent stood behind.
    let mut per_kind: HashMap<String, usize> = HashMap::new();
    let mut out = Links::default();
    for f in all {
        let n = per_kind.entry(f.item.kind.clone()).or_default();
        if out.top.len() < 5 && *n < 2 && f.score >= 40 {
            *n += 1;
            out.top.push(f.item);
        } else if out.more.len() < 20 {
            out.more.push(f.item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_common_links() {
        let l = |u: &str| classify(u, true).map(|i| (i.kind, i.label));
        assert_eq!(l("https://github.com/acme/shop/pull/6191"), Some(("PR".into(), "PR #6191 in shop".into())));
        assert_eq!(
            l("https://www.notion.so/acme/Support-initiative-PRD-0123456789abcdef0123456789abcdef"),
            Some(("Notion".into(), "Support initiative PRD".into()))
        );
        assert_eq!(l("https://linear.app/acme/issue/ENG-396/add-retry-backoff").unwrap().1, "ENG-396 Add retry backoff");
        assert_eq!(
            l("https://linear.app/acme/issue/ENG-5019/open-every-link-in-the-default-browser").unwrap().1,
            "ENG-5019 Open every link in the…"
        );
        assert_eq!(l("https://acme.atlassian.net/wiki/spaces/OP/pages/123/Email+series+revamp").unwrap().1, "Email series revamp");
        assert_eq!(l("https://docs.rs/regex"), None);
        assert_eq!(classify("https://docs.rs/regex", false).unwrap().kind, "Link");
        assert_eq!(l("http://localhost:5173/settings"), Some(("Preview".into(), "localhost:5173/settings".into())));
        assert_eq!(l("http://0.0.0.0:3000/"), Some(("Preview".into(), "localhost:3000".into())));
    }

    #[test]
    fn ranks_what_the_agent_reported() {
        let mid = |t: &str| serde_json::json!({"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"text","text":t}]}}).to_string();
        let end = |t: &str| serde_json::json!({"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":t}]}}).to_string();
        let lines = [
            mid("Looking at https://linear.app/a/issue/ENG-1/one and https://linear.app/a/issue/ENG-2/two and https://linear.app/a/issue/ENG-3/three"),
            end("Opened [the domain PR](https://github.com/acme/shop/pull/7) and wrote [Migration plan](https://www.notion.so/a/Plan-0123456789abcdef0123456789abcdef)."),
        ];
        let links = parse_links(&lines.join("\n"));
        let labels: Vec<&str> = links.top.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(labels, vec!["Migration plan", "the domain PR"]);
        assert!(links.more.is_empty(), "passing mentions are dropped");
    }
}
