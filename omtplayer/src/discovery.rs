use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type SourceList = Arc<RwLock<Vec<String>>>;

/// Start a background task that periodically discovers OMT sources via avahi-browse.
/// Returns a shared list of discovered source addresses (e.g. "omt://hostname:6400").
pub fn start_discovery() -> SourceList {
    let sources: SourceList = Arc::new(RwLock::new(Vec::new()));
    let sources_clone = sources.clone();

    tokio::spawn(async move {
        loop {
            let discovered = browse_omt_sources();
            {
                let mut list = sources_clone.write().await;
                *list = discovered;
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    sources
}

/// Run avahi-browse to find _omt._tcp services on the local network.
/// Returns a list of "omt://address:port" strings.
fn browse_omt_sources() -> Vec<String> {
    let output = Command::new("avahi-browse")
        .args(["-rpt", "_omt._tcp"])
        .output();

    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_avahi_output(&stdout)
}

/// Parse avahi-browse -rpt output into omt:// URLs.
/// Format: "=;interface;protocol;name;type;domain;hostname;address;port;txt"
fn parse_avahi_output(output: &str) -> Vec<String> {
    let mut sources: HashMap<String, String> = HashMap::new();

    for line in output.lines() {
        if !line.starts_with('=') {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        if fields.len() < 9 {
            continue;
        }

        let name = fields[3];
        let address = fields[7];
        let port = fields[8];

        // Prefer IPv4 (skip IPv6 link-local)
        if address.contains(':') && sources.contains_key(name) {
            continue;
        }

        let url = format!("omt://{}:{}", address, port);
        sources.insert(name.to_string(), url);
    }

    sources.into_values().collect()
}
