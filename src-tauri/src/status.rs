//! Orca's live agent-status cache (`agent-hooks/last-status.json`), keyed by pane (`tabId:leafId`).
//! Orca rewrites it on every Claude Code hook, so polling its mtime is enough to stay current.

use serde_json::Value;
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Clone, Debug, Default)]
pub struct PaneStatus {
    pub state: String,
    pub prompt: Option<String>,
    pub last_message: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub transcript_path: Option<String>,
    pub updated_at: i64,
}

#[derive(Default)]
pub struct StatusCache {
    mtime: Option<SystemTime>,
    pub panes: HashMap<String, PaneStatus>,
}

impl StatusCache {
    /// Re-reads the file only when it changed. Returns true if anything was reloaded.
    pub fn refresh(&mut self) -> bool {
        let path = crate::orca::orca_dir().join("agent-hooks/last-status.json");
        let Ok(meta) = std::fs::metadata(&path) else { return false };
        let mtime = meta.modified().ok();
        if mtime.is_some() && mtime == self.mtime {
            return false;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else { return false };
        // Orca may be mid-write; keep the old view and retry next tick.
        let Ok(doc) = serde_json::from_str::<Value>(&raw) else { return false };
        self.mtime = mtime;
        self.panes = doc["entries"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, e)| {
                let p = &e["payload"];
                let s = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
                (
                    key.clone(),
                    PaneStatus {
                        // A subagent finishing reports its own state; the main agent's is what matters.
                        state: s(&p["mainAgent"]["state"]).or_else(|| s(&p["state"])).unwrap_or_default(),
                        prompt: s(&p["prompt"]),
                        last_message: s(&p["lastAssistantMessage"]),
                        tool_name: s(&p["toolName"]),
                        tool_input: s(&p["toolInput"]),
                        transcript_path: s(&e["providerSession"]["transcriptPath"]),
                        updated_at: ["receivedAt", "evidenceObservedAt", "updatedAt"]
                            .iter()
                            .find_map(|k| e[*k].as_i64())
                            .unwrap_or(0),
                    },
                )
            })
            .collect();
        true
    }
}
