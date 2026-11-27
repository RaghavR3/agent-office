//! Turns an agent's final message into a short card: either the `brief` block the agent was asked
//! to write, or a best-effort fallback for sessions that weren't started by Agent Office.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Brief {
    pub status: String,
    pub headline: String,
    pub points: Vec<String>,
    pub questions: Vec<Question>,
    pub links: Vec<Link>,
    /// The agent's whole message, kept when it didn't write a brief, so nothing it said is lost.
    #[serde(skip_deserializing)]
    pub body: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Question {
    /// Agents write this as "q", "question", "text" or "prompt"; all mean the same.
    #[serde(alias = "question", alias = "text", alias = "prompt")]
    pub q: String,
    #[serde(deserialize_with = "option_labels")]
    pub options: Vec<String>,
}

/// Options as plain strings, or as objects with a label.
fn option_labels<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let raw: Vec<serde_json::Value> = Vec::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|o| match o {
            serde_json::Value::String(s) => Some(s),
            o => ["label", "text", "option"].iter().find_map(|k| o[*k].as_str().map(str::to_string)),
        })
        .collect())
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Link {
    pub label: String,
    pub url: String,
}

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https://[^\s)\]>"'`]+|http://(?:localhost|127\.0\.0\.1|0\.0\.0\.0)\b[^\s)\]>"'`]*"#).unwrap()
});
static PR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https://github\.com/[^/]+/([^/]+)/pull/(\d+)").unwrap());
static MD_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());

/// Returns the JSON body of the last ```<tag> fenced block, if any.
pub fn fenced(text: &str, tag: &str) -> Option<String> {
    let open = format!("```{tag}");
    let start = text.rfind(&open)? + open.len();
    let rest = &text[start..];
    let body_start = rest.find('\n').map(|i| i + 1).unwrap_or(0);
    let end = rest[body_start..].find("```").map(|i| body_start + i).unwrap_or(rest.len());
    Some(rest[body_start..end].trim().to_string())
}

/// All ```<tag> blocks, in order.
pub fn all_fenced(text: &str, tag: &str) -> Vec<String> {
    let open = format!("```{tag}");
    let mut out = vec![];
    let mut cursor = 0;
    while let Some(i) = text[cursor..].find(&open) {
        let start = cursor + i + open.len();
        let rest = &text[start..];
        let body_start = rest.find('\n').map(|i| i + 1).unwrap_or(0);
        let end = rest[body_start..].find("```").map(|i| body_start + i).unwrap_or(rest.len());
        out.push(rest[body_start..end].trim().to_string());
        cursor = start + end;
    }
    out
}

pub fn strip_fenced(text: &str, tag: &str) -> String {
    let open = format!("```{tag}");
    match text.find(&open) {
        Some(i) => {
            let after = &text[i + open.len()..];
            let close = after.find("\n```").map(|j| j + 4).unwrap_or(after.len());
            format!("{}{}", &text[..i], &after[close..]).trim().to_string()
        }
        None => text.trim().to_string(),
    }
}

pub fn from_message(text: &str) -> Brief {
    if let Some(body) = fenced(text, "brief") {
        if let Ok(mut b) = serde_json::from_str::<Brief>(&body) {
            if b.links.is_empty() {
                b.links = links_in(text);
            }
            if b.status.is_empty() {
                b.status = "done".into();
            }
            return b;
        }
    }
    fallback(text)
}

fn plain(text: &str) -> String {
    let t = MD_LINK.replace_all(text, "$1");
    t.replace("**", "").replace('`', "").replace("__", "")
}

fn first_sentence(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('|') && !l.starts_with("```") && !l.starts_with('#'))
        .unwrap_or("");
    let line = plain(line);
    let end = line
        .char_indices()
        .find(|&(i, c)| (c == '.' || c == '?' || c == '!') && line[i + 1..].starts_with(' '))
        .map(|(i, _)| i + 1)
        .unwrap_or(line.len());
    // Links show up as link buttons, so the headline keeps only the words.
    let sentence = URL.replace_all(&line[..end], "");
    clip(sentence.trim_end_matches([':', ' ']).trim(), max)
}

pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        cur.push(*c);
        if matches!(c, '.' | '?' | '!') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            out.push(cur.trim().to_string());
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Without a brief, the card leads with the opening sentence and any question in the closing
/// paragraph; the whole message stays one click away.
fn fallback(text: &str) -> Brief {
    let text = text.trim();
    let mut headline = first_sentence(text, 160);
    // "Done." on its own says too little, so take the sentence after it too.
    if headline.chars().count() < 30 {
        let first_para = text.split("\n\n").next().unwrap_or(text);
        if let Some(next) = sentences(&plain(first_para)).get(1).filter(|s| !s.ends_with('?')) {
            headline = clip(&format!("{headline} {}", URL.replace_all(next, "").trim()), 160);
        }
    }
    let mut b = Brief { status: "done".into(), headline, links: links_in(text), body: Some(text.to_string()), ..Default::default() };
    // A message whose closing paragraph asks something is waiting on the user.
    let last_para = text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).last().unwrap_or("");
    let asks: Vec<String> = sentences(&plain(last_para).replace('\n', " ")).into_iter().filter(|s| s.ends_with('?')).collect();
    if !asks.is_empty() {
        b.status = "needs_input".into();
        b.questions = asks.iter().rev().take(2).rev().map(|q| Question { q: clip(q.trim_start_matches(['-', '*', ' ']), 220), options: vec![] }).collect();
    }
    b
}

pub fn links_in(text: &str) -> Vec<Link> {
    let mut out: Vec<Link> = vec![];
    for m in URL.find_iter(text) {
        let url = m.as_str().trim_end_matches(['.', ',', ';', ':']).to_string();
        if out.iter().any(|l| l.url == url) {
            continue;
        }
        let label = if let Some(c) = PR.captures(&url) {
            format!("PR #{} in {}", &c[2], &c[1])
        } else {
            url.trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("link")
                .trim_start_matches("www.")
                .to_string()
        };
        out.push(Link { label, url });
        if out.len() == 4 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_brief_block() {
        let msg = "Done.\n\n```brief\n{\"status\":\"needs_input\",\"headline\":\"Pick a name\",\"questions\":[{\"q\":\"Which?\",\"options\":[\"A\",\"B\"]}]}\n```\n";
        let b = from_message(msg);
        assert_eq!(b.status, "needs_input");
        assert_eq!(b.headline, "Pick a name");
        assert_eq!(b.questions[0].options, vec!["A", "B"]);
    }

    #[test]
    fn reads_questions_however_they_are_written() {
        let b = from_message("```brief\n{\"status\":\"needs_input\",\"headline\":\"4 answers\",\"questions\":[{\"question\":\"Who reads this?\",\"options\":[\"External\",{\"label\":\"Internal\"}]}]}\n```");
        assert_eq!(b.questions[0].q, "Who reads this?");
        assert_eq!(b.questions[0].options, ["External", "Internal"]);
    }

    #[test]
    fn fallback_detects_trailing_question() {
        let b = from_message("I fixed the sync. It has the same flaw. Want me to fix that as well?");
        assert_eq!(b.status, "needs_input");
        assert_eq!(b.headline, "I fixed the sync. It has the same flaw.");
        assert_eq!(b.questions[0].q, "Want me to fix that as well?");
    }

    #[test]
    fn a_long_reply_leads_with_its_opening_line() {
        let msg = "All four are in the new build, which is running now. I haven't tried them live yet.\n\n**Questions.** The card now shows each question.\n\n- Sign in opens a terminal.\n- Reconnect waits for busy agents.";
        let b = from_message(msg);
        assert_eq!(b.headline, "All four are in the new build, which is running now.");
        assert!(b.questions.is_empty());
        assert_eq!(b.status, "done");
        assert_eq!(b.body.as_deref(), Some(msg));
    }

    #[test]
    fn headline_drops_urls() {
        let b = from_message("The fix is up as PR #6191: https://github.com/a/b/pull/6191. Want more?");
        assert_eq!(b.headline, "The fix is up as PR #6191");
    }

    #[test]
    fn labels_pr_links() {
        let b = from_message("Opened https://github.com/acme/widgets/pull/212. Thanks");
        assert_eq!(b.links[0].label, "PR #212 in widgets");
        assert_eq!(b.links[0].url, "https://github.com/acme/widgets/pull/212");
    }

    #[test]
    fn strips_brief_from_body() {
        let msg = "Hello\n```brief\n{}\n```\nbye";
        assert_eq!(strip_fenced(msg, "brief"), "Hello\n\nbye");
    }
}
