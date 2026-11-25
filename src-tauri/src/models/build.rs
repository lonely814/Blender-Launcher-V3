use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    pub id: String,
    pub version: String,
    pub branch: BuildBranch,
    pub fork: BuildFork,
    pub platform: Platform,
    pub architecture: Architecture,
    pub url: String,
    pub size_bytes: u64,
    pub release_date: DateTime<Utc>,
    pub release_notes_url: Option<String>,
    pub breaking_changes: Option<Vec<String>>,
    pub is_installed: bool,
    pub install_path: Option<PathBuf>,
    /// 分支列的显示文案。原版 `BuildInfo.display_label`：有自定义名称就用它，
    /// 否则按分支/变体生成一句人类可读的标签。`None` 表示用 `branch` 枚举名。
    #[serde(default)]
    pub label: Option<String>,
    /// 冻结更新（原版 `.blinfo` 的 `is_frozen`）：为真时永久不提示更新。
    #[serde(default)]
    pub is_frozen: bool,
    /// 同 fork/branch/平台存在更新的远程版本时，指向那个版本的 id。
    /// Launch 按钮据此变成 Update。
    #[serde(default)]
    pub update_build_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum BuildBranch {
    Stable,
    Daily,
    Experimental,
    Patch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum BuildFork {
    Official,
    Bforartists,
    Upbge,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    X64,
    Arm64,
}

impl Build {
    pub fn generate_id(version: &str, fork: BuildFork, platform: Platform) -> String {
        format!("{}-{:?}-{:?}", version, fork, platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_id_generation() {
        let id = Build::generate_id("4.2.0", BuildFork::Official, Platform::Linux);
        assert_eq!(id, "4.2.0-Official-Linux");
    }
}
