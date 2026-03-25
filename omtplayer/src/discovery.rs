use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

/// (display_name, omt://address:port)
pub type OmtSource = (String, String);
pub type SourceList = Arc<RwLock<Vec<OmtSource>>>;

/// Start a background task that periodically discovers OMT sources via avahi-browse.
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
fn browse_omt_sources() -> Vec<OmtSource> {
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

/// Parse avahi-browse -rpt output into (name, url) pairs.
/// Format: "=;interface;protocol;name;type;domain;hostname;address;port;txt"
fn parse_avahi_output(output: &str) -> Vec<OmtSource> {
    let mut sources: HashMap<String, OmtSource> = HashMap::new();

    for line in output.lines() {
        if !line.starts_with('=') {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        if fields.len() < 9 {
            continue;
        }

        let name = unescape_avahi(fields[3]);
        let address = fields[7];
        let port = fields[8];

        // Prefer IPv4 (skip IPv6 link-local)
        if address.contains(':') && sources.contains_key(&name) {
            continue;
        }

        let url = format!("omt://{}:{}", address, port);
        sources.insert(name.clone(), (name, url));
    }

    sources.into_values().collect()
}

/// Decode avahi-browse escaped strings like `stephen-mini\032\040OBS\032Output\041`
/// where `\DDD` is a decimal ASCII code.
fn unescape_avahi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            // Try \DDD decimal escape (exactly 3 digits)
            if i + 3 < bytes.len() {
                if let Ok(code) = std::str::from_utf8(&bytes[i + 1..i + 4])
                    .unwrap_or("")
                    .parse::<u8>()
                {
                    out.push(code as char);
                    i += 4;
                    continue;
                }
            }
            // Not a decimal escape — skip the backslash, keep the next char
            i += 1;
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
