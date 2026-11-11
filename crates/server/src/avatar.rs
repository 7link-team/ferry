//! Small account presentation data; no external URL, SVG, executable content or file path.
use crate::error::{ApiError, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Avatar {
    pub style: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}
impl Avatar {
    pub fn validate(&self) -> Result<()> {
        let invalid = || ApiError::localized(axum::http::StatusCode::BAD_REQUEST, crate::i18n::Message::new("request.jsonData"));
        if !["portrait", "pixel", "initials", "emoji", "image"].contains(&self.style.as_str())
            || self.seed.as_ref().is_some_and(|s| s.len() > 96 || s.chars().any(char::is_control))
            || self.emoji.as_ref().is_some_and(|s| s.is_empty() || s.len() > 64 || s.chars().any(char::is_control))
            || (self.style == "emoji" && self.emoji.is_none())
            || (self.style != "image" && self.image.is_some())
        { return Err(invalid()); }
        if self.style == "image" {
            let image = self.image.as_deref().filter(|s| s.len() <= 96 * 1024).ok_or_else(invalid)?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(image.strip_prefix("data:image/png;base64,").ok_or_else(invalid)?).map_err(|_| invalid())?;
            if bytes.len() < 33 || bytes[..8] != *b"\x89PNG\r\n\x1a\n" || bytes[12..16] != *b"IHDR" || bytes[16..20] != 128u32.to_be_bytes() || bytes[20..24] != 128u32.to_be_bytes() {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_a_real_128_png_and_rejects_wrong_dimensions() {
        let input = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAIAAAACACAYAAADDPmHLAAABUUlEQVR4nO3SMQHAIADAsDE/aMIaTkEGRxMFPTrm2ucj638dwFsGiDNAnAHiDBBngDgDxBkgzgBxBogzQJwB4gwQZ4A4A8QZIM4AcQaIM0CcAeIMEGeAOAPEGSDOAHEGiDNAnAHiDBBngDgDxBkgzgBxBogzQJwB4gwQZ4A4A8QZIM4AcQaIM0CcAeIMEGeAOAPEGSDOAHEGiDNAnAHiDBBngDgDxBkgzgBxBogzQJwB4gwQZ4A4A8QZIM4AcQaIM0CcAeIMEGeAOAPEGSDOAHEGiDNAnAHiDBBngDgDxBkgzgBxBogzQJwB4gwQZ4A4A8QZIM4AcQaIM0CcAeIMEGeAOAPEGSDOAHEGiDNAnAHiDBBngDgDxBkgzgBxBogzQJwB4gwQZ4A4A8QZIM4AcQaIM0CcAeIMEGeAOAPEGSDOAHEGiDNAnAHiDBBngDgDxF13XAMryrFmNAAAAABJRU5ErkJggg==";
        let avatar: Avatar = serde_json::from_value(serde_json::json!({"style":"image","image":input})).unwrap();
        avatar.validate().unwrap();
        let mut bytes = base64::engine::general_purpose::STANDARD.decode(input.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
        bytes[16..20].copy_from_slice(&256u32.to_be_bytes());
        let wrong = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
        let avatar: Avatar = serde_json::from_value(serde_json::json!({"style":"image","image":wrong})).unwrap();
        assert!(avatar.validate().is_err());
    }
    #[test]
    fn only_bounded_local_presentation_is_accepted() {
        for value in [serde_json::json!({"style":"pixel","seed":"stable"}),serde_json::json!({"style":"emoji","emoji":"🧑‍💻"})] {
            serde_json::from_value::<Avatar>(value).unwrap().validate().unwrap();
        }
        for value in [serde_json::json!({"style":"external"}),serde_json::json!({"style":"emoji"}),serde_json::json!({"style":"image","image":"https://example.invalid/image.png"}),serde_json::json!({"style":"image","image":"data:image/svg+xml;base64,AA=="}),serde_json::json!({"style":"pixel","image":"hidden"}),serde_json::json!({"style":"pixel","seed":"\n"})] {
            assert!(serde_json::from_value::<Avatar>(value).unwrap().validate().is_err());
        }
    }
}
