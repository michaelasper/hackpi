use serde::{Deserialize, Serialize};

// ── Response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub login: String,
    pub id: u64,
    pub avatar_url: Option<String>,
    pub html_url: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub body: Option<String>,
    pub head: PrRef,
    pub base: PrRef,
    pub draft: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
    pub user: PrUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrRef {
    pub label: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrUser {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub body: Option<String>,
    pub labels: Vec<IssueLabel>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLabel {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelInfo {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub name: String,
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    pub created_at: String,
    pub published_at: Option<String>,
}
