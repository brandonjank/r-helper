//! On-demand, model-scoped catalog checks; never downloads or runs installers.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub const SOURCE: &str = "https://mysupport.razer.com/app/answers/detail/a_id/19866";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    component: String,
    version: String,
    released: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    model: String,
    checked_unix: Option<u64>,
    verified: String,
    entries: Vec<Entry>,
}
fn bundled() -> Catalog {
    Catalog {
        model: "RZ09-0529".into(),
        checked_unix: None,
        verified: "2026-10-14 · bundled reference, not a live check".into(),
        entries: [
            ("BIOS", "4.01", "2026-06-30"),
            ("EC", "1.06", "2026-06-30"),
            ("MCU", "1.07", "2026-07-01"),
            ("VBIOS RTX 5090", "98.03.5C.40.25", "2026-06-30"),
            ("PD", "2v00.14", "2025-09-17"),
        ]
        .into_iter()
        .map(|(c, v, d)| Entry { component: c.into(), version: v.into(), released: d.into() })
        .collect(),
    }
}
fn cache_path() -> Result<PathBuf> {
    Ok(crate::blade_profiles::path()?.with_file_name("updates-catalog.json"))
}
fn valid(c: &Catalog) -> bool {
    c.model == "RZ09-0529"
        && c.entries.len() == 5
        && c.verified.len() < 128
        && bundled().entries.iter().all(|expected| {
            c.entries.iter().filter(|e| e.component == expected.component).count() == 1
        })
        && c.entries.iter().all(|e| {
            e.version.len() <= 40
                && e.released.len() <= 80
                && parts(&e.version, e.component.starts_with("VBIOS")).is_some()
        })
}
fn cached() -> Catalog {
    let read = || -> Result<Catalog> {
        let path = cache_path()?;
        ensure!(std::fs::metadata(&path)?.len() < 65536, "Catalog cache too large");
        let c: Catalog = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(valid(&c), "Invalid catalog cache");
        Ok(c)
    };
    read().unwrap_or_else(|_| bundled())
}
fn parts(text: &str, hex: bool) -> Option<Vec<u32>> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let pieces = text.split(['.', 'v', 'V']).collect::<Vec<_>>();
    if pieces.len() < 2 || pieces.len() > 8 {
        return None;
    }
    let mut values = pieces
        .into_iter()
        .map(|v| u32::from_str_radix(v, if hex { 16 } else { 10 }).ok())
        .collect::<Option<Vec<_>>>()?;
    while values.len() > 2 && values.last() == Some(&0) {
        values.pop();
    }
    Some(values)
}
fn comparison(installed: Option<&str>, available: &str, hex: bool) -> &'static str {
    let Some(a) = installed.and_then(|v| parts(v, hex)) else {
        return "Installed version unavailable";
    };
    let Some(b) = parts(available, hex) else { return "Check official page" };
    if a == b {
        "Matches catalog"
    } else if a < b {
        "Newer version listed · review applicability"
    } else {
        "Installed version newer than catalog"
    }
}
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                tag = true;
                out.push(' ');
            }
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn parse(html: &str) -> Result<Catalog> {
    ensure!(html.len() < 2_097_152, "Catalog page too large");
    let plain = text(html);
    ensure!(
        plain.contains("RZ09-0529") && plain.contains("Drivers") && plain.contains("Downloads"),
        "Official catalog is unavailable or model mismatch; retaining previous catalog"
    );
    let lower = html.to_ascii_lowercase();
    let mut c = bundled();
    let mut found = std::collections::HashSet::new();
    for (start, _) in lower.match_indices("<tr") {
        let Some(end) = lower[start..].find("</tr>") else { continue };
        let row = &html[start..start + end];
        let row_lower = row.to_ascii_lowercase();
        let cells = row_lower
            .match_indices("<td")
            .filter_map(|(offset, _)| {
                let open = row_lower[offset..].find('>')? + offset + 1;
                let close = row_lower[open..].find("</td>")? + open;
                Some(text(&row[open..close]))
            })
            .collect::<Vec<_>>();
        if cells.len() < 3 {
            continue;
        }
        let component = if cells[0].starts_with("BIOS ") {
            "BIOS"
        } else if cells[0].starts_with("EC ") {
            "EC"
        } else if cells[0].starts_with("MCU ") {
            "MCU"
        } else if cells[0].starts_with("PD ") {
            "PD"
        } else if cells[0].to_lowercase().starts_with("vbios ") {
            "VBIOS RTX 5090"
        } else {
            continue;
        };
        let version = if component.starts_with("VBIOS") {
            cells[1]
                .split("RTX 5090:")
                .nth(1)
                .context("RTX 5090 catalog entry missing")?
                .split_whitespace()
                .next()
                .context("Missing VBIOS version")?
                .to_owned()
        } else {
            cells[1].clone()
        };
        ensure!(found.insert(component), "Duplicate catalog component");
        let entry = c.entries.iter_mut().find(|e| e.component == component).unwrap();
        entry.version = version;
        entry.released = cells[2].clone();
    }
    ensure!(
        found.len() == 5 && valid(&c),
        "Incomplete or changed catalog format; retaining previous catalog"
    );
    c.checked_unix = Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs());
    c.verified = "Live official model catalog".into();
    Ok(c)
}
fn fetch() -> Result<Catalog> {
    let output = crate::blade_profiles::journal_path("updates-source")?;
    let exe = PathBuf::from(std::env::var_os("SystemRoot").context("Windows unavailable")?)
        .join("System32/curl.exe");
    let mut command = std::process::Command::new(exe);
    command
        .args([
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--location",
            "--max-redirs",
            "3",
            "--tlsv1.2",
            "--max-time",
            "15",
            "--max-filesize",
            "2097152",
            "--fail",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&output)
        .arg(SOURCE)
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Official update check failed; retaining previous catalog");
            break;
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Update check timed out; retaining previous catalog");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ensure!(std::fs::metadata(&output)?.len() < 2_097_152, "Catalog page too large");
    let c = parse(&std::fs::read_to_string(output)?)?;
    let path = cache_path()?;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let file = std::fs::File::options().create_new(true).write(true).open(&tmp)?;
    serde_json::to_writer_pretty(&file, &c)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    Ok(c)
}
pub fn inspect(refresh: bool) -> Result<Value> {
    ensure!(librazer::diagnostics::is_target_system(), "Updates are model scoped");
    let mut catalog = cached();
    let mut error = None;
    if refresh {
        match fetch() {
            Ok(c) => catalog = c,
            Err(e) => error = Some(format!("{e:#}")),
        }
    }
    let firmware = crate::blade_platform::isolated_getter("GetBladeFirmwareInfo");
    let source = firmware.as_ref().ok().map(|v| &v["value"]);
    let drivers = crate::blade_platform::windows_drivers();
    let rtx5090 =
        drivers.as_ref().ok().and_then(|v| v["installed"].as_array()).is_some_and(|rows| {
            rows.iter().any(|v| v["name"].as_str().is_some_and(|n| n.contains("5090")))
        });
    let rows = catalog.entries.iter().map(|e| {
        let key = match e.component.as_str(){"BIOS"=>"BiosFW","EC"=>"ECFW","PD"=>"PDFW","VBIOS RTX 5090" if rtx5090=>"VBiosFW",_=>""};
        let installed = source.and_then(|s|s[key].as_str());
        json!({"component":e.component,"installed":installed,"available":e.version,"released":e.released,"status":comparison(installed,&e.version,e.component.starts_with("VBIOS")),"source":SOURCE,"notes":if e.component=="PD"{"Official notes: USB-C port update and Leviathan V2 X power fix."}else{"Open official release information and review the exact SKU before updating."}})
    }).collect::<Vec<_>>();
    Ok(
        json!({"model":"RZ09-0529","catalog":catalog,"check_error":error,"firmware_error":firmware.err().map(|e|format!("{e:#}")),"rows":rows,"drivers":drivers.unwrap_or_else(|e|json!({"error":format!("{e:#}")})),"driver_sources":[{"name":"NVIDIA drivers","url":"https://www.nvidia.com/en-us/drivers/"},{"name":"Intel drivers","url":"https://www.intel.com/content/www/us/en/download-center/home.html"}],"note":"Catalog comparison only; no automatic downloads, installation or flashing. Driver latest versions require manual selection on the official sites."}),
    )
}
pub fn dispatch() -> Option<Result<()>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mode = args.get(1)?;
    if mode != "--updates-inspect" && mode != "--updates-refresh" {
        return None;
    }
    Some((|| {
        ensure!(args.len() == 3, "Expected output path");
        let result = inspect(mode == "--updates-refresh")?;
        let file =
            std::fs::File::options().create_new(true).write(true).open(Path::new(&args[2]))?;
        serde_json::to_writer_pretty(&file, &result)?;
        file.sync_all()?;
        Ok(())
    })())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_comparison_handles_hex_padding_and_unknowns() {
        assert_eq!(comparison(Some("98.3.5c.40.25"), "v98.03.5C.40.25", true), "Matches catalog");
        assert_eq!(
            comparison(Some("2.01"), "4.01", false),
            "Newer version listed · review applicability"
        );
        assert_eq!(comparison(None, "1.07", false), "Installed version unavailable");
        assert_eq!(comparison(Some("unknown"), "1.07", false), "Installed version unavailable");
        assert!(parts("2v00.14", false).is_some());
    }
    #[test]
    fn parser_requires_whole_model_catalog_and_selects_5090() {
        let mut html = "RZ09-0529 Drivers Downloads <table>".to_owned();
        for e in bundled().entries {
            let name = if e.component.starts_with("VBIOS") { "vBIOS" } else { &e.component };
            let version = if name == "vBIOS" {
                "RTX 5080: v98.03.5C.40.26<br>RTX 5090: v98.03.5C.40.25"
            } else {
                &e.version
            };
            html.push_str(&format!(
                "<tr><td>{name} Firmware Updater</td><td>{version}</td><td>{}</td></tr>",
                e.released
            ));
        }
        let c = parse(&html).unwrap();
        assert_eq!(c.entries[3].version, "v98.03.5C.40.25");
        assert!(c.checked_unix.is_some());
        assert!(parse(&html.replace("RZ09-0529", "RZ09-0528")).is_err());
        assert!(parse("RZ09-0529 Drivers Downloads <html>Access denied</html>").is_err());
    }
}
