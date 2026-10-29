//! CurseForge API data models: projects, files, categories, and the
//! `{data, pagination}` response envelope.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfAuthor {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "avatarUrl")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfLogo {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, rename = "thumbnailUrl")]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfCategoryRef {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfLinks {
    #[serde(default)]
    pub website_url: Option<String>,
    #[serde(default)]
    pub wiki_url: Option<String>,
    #[serde(default)]
    pub issues_url: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfScreenshot {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfLatestFileIndex {
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub file_id: u64,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub release_type: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfProject {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub thumbs_up_count: Option<u64>,
    #[serde(default)]
    pub date_created: Option<String>,
    #[serde(default)]
    pub date_modified: Option<String>,
    #[serde(default)]
    pub date_released: Option<String>,
    #[serde(default)]
    pub authors: Vec<CfAuthor>,
    #[serde(default)]
    pub logo: Option<CfLogo>,
    #[serde(default)]
    pub categories: Vec<CfCategoryRef>,
    #[serde(default)]
    pub links: Option<CfLinks>,
    #[serde(default)]
    pub class_id: Option<u64>,
    #[serde(default)]
    pub main_file_id: Option<u64>,
    #[serde(default)]
    pub latest_files_indexes: Vec<CfLatestFileIndex>,
    #[serde(default)]
    pub screenshots: Vec<CfScreenshot>,
    #[serde(default)]
    pub is_available: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CfDependency {
    #[serde(default)]
    pub mod_id: u64,
    /// 1 embedded, 2 optional, 3 required, 4 tool, 5 incompatible, 6 include.
    #[serde(default)]
    pub relation_type: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfFile {
    pub id: u64,
    #[serde(default)]
    pub mod_id: u64,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub file_name: String,
    /// 1 release, 2 beta, 3 alpha.
    #[serde(default)]
    pub release_type: u8,
    #[serde(default)]
    pub file_date: Option<String>,
    #[serde(default)]
    pub file_length: u64,
    #[serde(default)]
    pub download_count: u64,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub is_available: bool,
    #[serde(default)]
    pub dependencies: Vec<CfDependency>,
}

impl CfFile {
    pub fn release_label(&self) -> &'static str {
        match self.release_type {
            1 => "release",
            2 => "beta",
            3 => "alpha",
            _ => "unknown",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfPagination {
    #[serde(default)]
    pub index: u64,
    #[serde(default)]
    pub page_size: u64,
    #[serde(default)]
    pub total_count: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfCategory {
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub parent_category_id: Option<u64>,
    #[serde(default)]
    pub class_id: Option<u64>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Envelope<T> {
    pub(super) data: T,
    #[serde(default)]
    pub(super) pagination: Option<CfPagination>,
}
