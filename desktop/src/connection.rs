use std::path::Path;

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Preferences {
    pub server_url: String,
}

pub(crate) fn read_preferences(data: &Path) -> Result<Option<Preferences>, String> {
    // Prefer the durable selection; the marker also migrates older updater resumes.
    for name in ["connection.json", "update-resume.json"] {
        match std::fs::read(data.join(name)) {
            Ok(bytes) => {
                return serde_json::from_slice(&bytes)
                    .map(Some)
                    .map_err(|e| format!("无法读取已保存的连接设置，请重新选择：{e}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("无法读取连接设置：{e}")),
        }
    }
    Ok(None)
}
pub(crate) fn save_preferences(data: &Path, settings: &Preferences) -> Result<(), String> {
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    std::fs::write(
        data.join("connection.json"),
        serde_json::to_vec(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("无法保存连接设置：{e}"))?;
    let _ = std::fs::remove_file(data.join("update-resume.json"));
    Ok(())
}
