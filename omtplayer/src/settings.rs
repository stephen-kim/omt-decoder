use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub source: String,
    pub audio_devices: Vec<String>,
    pub volume: f32,
    pub web_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: "None".to_string(),
            audio_devices: Vec::new(),
            volume: 1.0,
            web_port: 8080,
        }
    }
}

impl Settings {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let settings: Settings = serde_json::from_str(&content)?;
        Ok(settings)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Migrate from legacy settings.xml (C# format)
    #[allow(dead_code)]
    pub fn load_from_xml<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let mut settings = Settings::default();

        if let Some(source) = extract_xml_value(&content, "Source") {
            settings.source = source;
        }
        if let Some(devices) = extract_xml_value(&content, "AudioDevices") {
            settings.audio_devices = devices
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(port) = extract_xml_value(&content, "WebServerPort") {
            settings.web_port = port.parse().unwrap_or(8080);
        }

        Ok(settings)
    }
}

#[allow(dead_code)]
fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}
