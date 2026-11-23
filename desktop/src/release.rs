//! Shared release configuration for native clients.
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Channel {
    pub repository: String,
    #[serde(default)]
    pub include_prereleases: bool,
    #[serde(default)]
    pub mobile: MobileChannel,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MobileChannel {
    pub package_name: String,
    pub ios_distribution_url: Option<String>,
}
#[derive(Deserialize)]
pub(crate) struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub body: Option<String>,
}
#[derive(Deserialize)]
pub(crate) struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub size: u64,
}
pub(crate) fn channel(app: &tauri::AppHandle) -> Result<Channel, String> {
    let channel: Channel = serde_json::from_value(
        app.config()
            .plugins
            .0
            .get("release")
            .cloned()
            .ok_or("未配置更新来源。")?,
    )
    .map_err(|_| "更新来源配置无效。")?;
    let parts: Vec<_> = channel.repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return Err("更新仓库配置无效。".into());
    }
    Ok(channel)
}
