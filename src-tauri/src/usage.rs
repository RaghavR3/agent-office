//! Claude plan usage and saved accounts, as Orca reports them (`accounts.list`). Orca holds the
//! logins and asks Anthropic for the numbers; the office only reads its summary.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    /// Percent of the limit used, 0 to 100.
    pub used: f64,
    pub resets_at: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// The rolling five-hour limit.
    pub session: Option<Window>,
    pub week: Option<Window>,
    /// Fable has a weekly limit of its own.
    pub fable: Option<Window>,
    pub updated_at: i64,
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    pub active: bool,
    pub limits: Option<Limits>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Accounts saved in Orca, if you've added more than the one Claude Code is signed in with.
    pub accounts: Vec<Account>,
    /// The account in use now.
    pub limits: Option<Limits>,
}

fn window(w: &Value) -> Option<Window> {
    Some(Window { used: w["usedPercent"].as_f64()?.clamp(0.0, 100.0), resets_at: w["resetsAt"].as_i64().unwrap_or(0) })
}

fn limits(rl: &Value) -> Option<Limits> {
    if !rl.is_object() {
        return None;
    }
    Some(Limits {
        session: window(&rl["session"]),
        week: window(&rl["weekly"]),
        fable: window(&rl["fableWeekly"]),
        updated_at: rl["updatedAt"].as_i64().unwrap_or(0),
        error: rl["error"].as_str().filter(|e| !e.is_empty()).map(str::to_string),
    })
}

/// Plan usage from Claude's own status-line data, which the office's own terminals save.
/// Its reset times are in seconds.
pub fn from_status_line(v: &Value, at_ms: i64) -> Option<Usage> {
    let rl = &v["rate_limits"];
    let w = |x: &Value| Some(Window { used: x["used_percentage"].as_f64()?.clamp(0.0, 100.0), resets_at: x["resets_at"].as_i64().unwrap_or(0) * 1000 });
    let limits = Limits { session: w(&rl["five_hour"]), week: w(&rl["seven_day"]), fable: None, updated_at: at_ms, error: None };
    (limits.session.is_some() || limits.week.is_some()).then(|| Usage { accounts: vec![], limits: Some(limits) })
}

/// "Acme", or "Personal" for the organisation Claude makes for an individual plan.
fn account_name(org: &str, email: &str) -> String {
    let org = org.trim();
    if org.is_empty() || org == email {
        email.to_string()
    } else if org.ends_with("'s Organization") || org.ends_with("’s Organization") {
        "Personal".into()
    } else {
        org.to_string()
    }
}

pub fn parse(v: &Value) -> Usage {
    let active = v["claude"]["activeAccountId"].as_str().unwrap_or_default();
    let others: HashMap<&str, &Value> = v["rateLimits"]["inactiveClaudeAccounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["accountId"].as_str()?, &a["rateLimits"])))
        .collect();
    let now = limits(&v["rateLimits"]["claude"]);
    let accounts = v["claude"]["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let id = a["id"].as_str()?.to_string();
            let email = a["email"].as_str().unwrap_or_default().to_string();
            let is_active = id == active;
            Some(Account {
                name: account_name(a["organizationName"].as_str().unwrap_or_default(), &email),
                limits: if is_active { now.clone() } else { others.get(id.as_str()).and_then(|r| limits(r)) },
                active: is_active,
                email,
                id,
            })
        })
        .collect();
    Usage { accounts, limits: now }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_usage_from_the_status_line() {
        let v = serde_json::json!({ "rate_limits": { "five_hour": { "used_percentage": 22, "resets_at": 1795767600 }, "seven_day": { "used_percentage": 26, "resets_at": 1796122800 } } });
        let l = from_status_line(&v, 5).unwrap().limits.unwrap();
        assert_eq!(l.session, Some(Window { used: 22.0, resets_at: 1_795_767_600_000 }));
        assert_eq!(l.week.map(|w| w.used), Some(26.0));
        assert_eq!(l.updated_at, 5);
        assert!(from_status_line(&serde_json::json!({ "model": {} }), 5).is_none(), "no limits, no bar");
    }
    use serde_json::json;

    #[test]
    fn reads_orcas_accounts() {
        let v = json!({
            "claude": {
                "accounts": [
                    { "id": "a1", "email": "sam@example.com", "organizationName": "Acme" },
                    { "id": "a2", "email": "sam@example.com", "organizationName": "sam@example.com's Organization" }
                ],
                "activeAccountId": "a1"
            },
            "rateLimits": {
                "claude": {
                    "session": { "usedPercent": 47, "resetsAt": 100 },
                    "weekly": { "usedPercent": 36, "resetsAt": 200 },
                    "fableWeekly": null,
                    "updatedAt": 5,
                    "error": null
                },
                "inactiveClaudeAccounts": [{ "accountId": "a2", "rateLimits": { "session": { "usedPercent": 3, "resetsAt": 9 } } }]
            }
        });
        let u = parse(&v);
        assert_eq!(u.accounts.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Acme", "Personal"]);
        assert!(u.accounts[0].active && !u.accounts[1].active);
        let now = u.limits.unwrap();
        assert_eq!(now.session, Some(Window { used: 47.0, resets_at: 100 }));
        assert_eq!(now.fable, None);
        assert_eq!(u.accounts[1].limits.as_ref().unwrap().session.as_ref().unwrap().used, 3.0);
    }
}
