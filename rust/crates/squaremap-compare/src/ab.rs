use crate::benchmark::percentile;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AbSampleFile {
    pub backend: String,
    pub workload: String,
    pub case_count: u64,
    pub warmup_passes: u64,
    pub measured_passes: u64,
    pub elapsed_nanos: u64,
    pub items_per_second: f64,
    pub checksum: i64,
    pub manifest_hash: String,
    pub pass_nanos: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AbBackendStats {
    pub backend: String,
    pub min_nanos: u64,
    pub median_nanos: u64,
    pub p95_nanos: u64,
    pub max_nanos: u64,
    pub items_per_second_median: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AbReport {
    pub workload: String,
    pub case_count: u64,
    pub warmup_passes: u64,
    pub measured_passes: u64,
    pub manifest_hash: String,
    pub java_checksum: i64,
    pub rust_checksum: i64,
    pub passed: bool,
    pub java: AbBackendStats,
    pub rust: AbBackendStats,
    pub speedup: f64,
}

#[derive(Debug)]
pub enum AbError {
    Io(io::Error),
    Json(serde_json::Error),
    Validation(String),
}

impl std::fmt::Display for AbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Validation(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AbError {}
impl From<io::Error> for AbError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for AbError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub fn load_sample(path: impl AsRef<Path>) -> Result<AbSampleFile, AbError> {
    let sample: AbSampleFile = serde_json::from_slice(&fs::read(path)?)?;
    sample.validate()?;
    Ok(sample)
}

impl AbSampleFile {
    fn validate(&self) -> Result<(), AbError> {
        if self.pass_nanos.is_empty() {
            return Err(AbError::Validation("pass_nanos must not be empty".into()));
        }
        if self.pass_nanos.len() as u64 != self.measured_passes {
            return Err(AbError::Validation(format!(
                "pass_nanos length {} != measured_passes {}",
                self.pass_nanos.len(),
                self.measured_passes
            )));
        }
        Ok(())
    }
}

pub fn build_report(java: &AbSampleFile, rust: &AbSampleFile) -> Result<AbReport, AbError> {
    java.validate()?;
    rust.validate()?;
    reject_mismatch("workload", &java.workload, &rust.workload)?;
    reject_mismatch(
        "case_count",
        &java.case_count.to_string(),
        &rust.case_count.to_string(),
    )?;
    reject_mismatch(
        "warmup_passes",
        &java.warmup_passes.to_string(),
        &rust.warmup_passes.to_string(),
    )?;
    reject_mismatch(
        "measured_passes",
        &java.measured_passes.to_string(),
        &rust.measured_passes.to_string(),
    )?;
    reject_mismatch("manifest_hash", &java.manifest_hash, &rust.manifest_hash)?;

    let java_stats = backend_stats(java)?;
    let rust_stats = backend_stats(rust)?;
    let speedup = rust_stats.items_per_second_median / java_stats.items_per_second_median;
    Ok(AbReport {
        workload: java.workload.clone(),
        case_count: java.case_count,
        warmup_passes: java.warmup_passes,
        measured_passes: java.measured_passes,
        manifest_hash: java.manifest_hash.clone(),
        java_checksum: java.checksum,
        rust_checksum: rust.checksum,
        passed: java.checksum == rust.checksum,
        java: java_stats,
        rust: rust_stats,
        speedup,
    })
}

fn reject_mismatch(field: &str, java: &str, rust: &str) -> Result<(), AbError> {
    if java == rust {
        Ok(())
    } else {
        Err(AbError::Validation(format!(
            "{field} mismatch: java={java} rust={rust}"
        )))
    }
}

fn backend_stats(sample: &AbSampleFile) -> Result<AbBackendStats, AbError> {
    let mut samples: Vec<u128> = sample.pass_nanos.iter().copied().map(u128::from).collect();
    let min_nanos = required_percentile(&mut samples, 0.0, "min")?;
    let median_nanos = required_percentile(&mut samples, 0.5, "median")?;
    let p95_nanos = required_percentile(&mut samples, 0.95, "p95")?;
    let max_nanos = required_percentile(&mut samples, 1.0, "max")?;
    let items_per_second_median = sample.case_count as f64 * 1e9 / median_nanos as f64;
    Ok(AbBackendStats {
        backend: sample.backend.clone(),
        min_nanos,
        median_nanos,
        p95_nanos,
        max_nanos,
        items_per_second_median,
    })
}

fn required_percentile(samples: &mut [u128], rank: f64, label: &str) -> Result<u64, AbError> {
    percentile(samples, rank)
        .map(|value| value as u64)
        .ok_or_else(|| AbError::Validation(format!("unable to compute {label}")))
}

pub fn write_ab_report(
    output_dir: impl AsRef<Path>,
    java: &AbSampleFile,
    rust: &AbSampleFile,
    report: &AbReport,
) -> Result<(), AbError> {
    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir)?;
    fs::write(
        output_dir.join("ab-report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    fs::write(output_dir.join("throughput.svg"), throughput_svg(report))?;
    fs::write(
        output_dir.join("pass-times.svg"),
        pass_times_svg(java, rust, report),
    )?;
    fs::write(output_dir.join("index.html"), index_html(report))?;
    Ok(())
}

pub fn generate_ab_report(
    java_path: impl AsRef<Path>,
    rust_path: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
) -> Result<AbReport, AbError> {
    let java = load_sample(java_path)?;
    let rust = load_sample(rust_path)?;
    let report = build_report(&java, &rust)?;
    write_ab_report(output_dir, &java, &rust, &report)?;
    Ok(report)
}

fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn throughput_svg(report: &AbReport) -> String {
    let workload = escape_xml(&report.workload);
    let max_rate = report
        .java
        .items_per_second_median
        .max(report.rust.items_per_second_median)
        .max(1.0);
    let plot_top = 56.0;
    let plot_bottom = 300.0;
    let plot_height = plot_bottom - plot_top;
    let java_h = report.java.items_per_second_median / max_rate * plot_height;
    let rust_h = report.rust.items_per_second_median / max_rate * plot_height;
    let java_y = plot_bottom - java_h;
    let rust_y = plot_bottom - rust_h;
    format!(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="960" height="360" viewBox="0 0 960 360">"##,
            r##"<rect width="960" height="360" fill="#ffffff"/>"##,
            r##"<text x="480" y="32" text-anchor="middle" font-family="sans-serif" font-size="18">{workload} median items/s</text>"##,
            r##"<rect x="260" y="{java_y:.2}" width="140" height="{java_h:.2}" fill="#4C78A8"/>"##,
            r##"<text x="330" y="328" text-anchor="middle" font-family="sans-serif" font-size="16">java</text>"##,
            r##"<rect x="560" y="{rust_y:.2}" width="140" height="{rust_h:.2}" fill="#F58518"/>"##,
            r##"<text x="630" y="328" text-anchor="middle" font-family="sans-serif" font-size="16">rust</text>"##,
            r##"<text x="330" y="{java_label:.2}" text-anchor="middle" font-family="sans-serif" font-size="12">{java_rate}</text>"##,
            r##"<text x="630" y="{rust_label:.2}" text-anchor="middle" font-family="sans-serif" font-size="12">{rust_rate}</text>"##,
            "</svg>\n"
        ),
        workload = workload,
        java_y = java_y,
        java_h = java_h,
        rust_y = rust_y,
        rust_h = rust_h,
        java_label = (java_y - 8.0).max(48.0),
        rust_label = (rust_y - 8.0).max(48.0),
        java_rate = format_rate(report.java.items_per_second_median),
        rust_rate = format_rate(report.rust.items_per_second_median),
    )
}

fn pass_times_svg(java: &AbSampleFile, rust: &AbSampleFile, report: &AbReport) -> String {
    let workload = escape_xml(&report.workload);
    let n = java.pass_nanos.len().max(1);
    let y_max = java
        .pass_nanos
        .iter()
        .chain(&rust.pass_nanos)
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let left = 64.0;
    let right = 900.0;
    let top = 56.0;
    let bottom = 300.0;
    let width = right - left;
    let height = bottom - top;
    let denom = (n - 1).max(1) as f64;
    let java_points = polyline_points(&java.pass_nanos, left, width, top, height, denom, y_max);
    let rust_points = polyline_points(&rust.pass_nanos, left, width, top, height, denom, y_max);
    format!(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="960" height="360" viewBox="0 0 960 360">"##,
            r##"<rect width="960" height="360" fill="#ffffff"/>"##,
            r##"<text x="480" y="32" text-anchor="middle" font-family="sans-serif" font-size="18">{workload} pass times</text>"##,
            r##"<line x1="{left}" y1="{top}" x2="{left}" y2="{bottom}" stroke="#333333"/>"##,
            r##"<line x1="{left}" y1="{bottom}" x2="{right}" y2="{bottom}" stroke="#333333"/>"##,
            r##"<polyline fill="none" stroke="#4C78A8" stroke-width="2" points="{java_points}"/>"##,
            r##"<polyline fill="none" stroke="#F58518" stroke-width="2" points="{rust_points}"/>"##,
            r##"<rect x="720" y="48" width="12" height="12" fill="#4C78A8"/>"##,
            r##"<text x="740" y="58" font-family="sans-serif" font-size="14">java</text>"##,
            r##"<rect x="800" y="48" width="12" height="12" fill="#F58518"/>"##,
            r##"<text x="820" y="58" font-family="sans-serif" font-size="14">rust</text>"##,
            "</svg>\n"
        ),
        workload = workload,
        left = left,
        top = top,
        bottom = bottom,
        right = right,
        java_points = java_points,
        rust_points = rust_points,
    )
}

fn polyline_points(
    samples: &[u64],
    left: f64,
    width: f64,
    top: f64,
    height: f64,
    denom: f64,
    y_max: f64,
) -> String {
    samples
        .iter()
        .enumerate()
        .map(|(index, nanos)| {
            let x = left + index as f64 / denom * width;
            let y = top + height - (*nanos as f64 / y_max * height);
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_rate(value: f64) -> String {
    if value >= 1000.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

fn index_html(report: &AbReport) -> String {
    let banner = if report.passed {
        String::new()
    } else {
        concat!(
            r#"<div style="background:#b00020;color:#ffffff;padding:12px;font-weight:bold;">"#,
            "CHECKSUM MISMATCH — do not publish a speedup",
            "</div>\n"
        )
        .into()
    };
    format!(
        concat!(
            "<!DOCTYPE html>\n",
            "<html lang=\"en\">\n",
            "<head><meta charset=\"utf-8\"><title>{workload} A/B report</title></head>\n",
            "<body>\n",
            "{banner}",
            "<h1>{workload}</h1>\n",
            "<p>passed: {passed}</p>\n",
            "<table>\n",
            "<tr><th></th><th>java</th><th>rust</th></tr>\n",
            "<tr><td>min</td><td>{java_min}</td><td>{rust_min}</td></tr>\n",
            "<tr><td>median</td><td>{java_median}</td><td>{rust_median}</td></tr>\n",
            "<tr><td>p95</td><td>{java_p95}</td><td>{rust_p95}</td></tr>\n",
            "<tr><td>max</td><td>{java_max}</td><td>{rust_max}</td></tr>\n",
            "<tr><td>items/s</td><td>{java_rate}</td><td>{rust_rate}</td></tr>\n",
            "<tr><td>checksum</td><td>{java_checksum}</td><td>{rust_checksum}</td></tr>\n",
            "<tr><td>passed</td><td colspan=\"2\">{passed}</td></tr>\n",
            "<tr><td>speedup</td><td colspan=\"2\">{speedup}</td></tr>\n",
            "</table>\n",
            "<p><img src=\"throughput.svg\" alt=\"throughput\"></p>\n",
            "<p><img src=\"pass-times.svg\" alt=\"pass times\"></p>\n",
            "</body>\n",
            "</html>\n"
        ),
        workload = escape_xml(&report.workload),
        banner = banner,
        passed = report.passed,
        java_min = report.java.min_nanos,
        rust_min = report.rust.min_nanos,
        java_median = report.java.median_nanos,
        rust_median = report.rust.median_nanos,
        java_p95 = report.java.p95_nanos,
        rust_p95 = report.rust.p95_nanos,
        java_max = report.java.max_nanos,
        rust_max = report.rust.max_nanos,
        java_rate = report.java.items_per_second_median,
        rust_rate = report.rust.items_per_second_median,
        java_checksum = report.java_checksum,
        rust_checksum = report.rust_checksum,
        speedup = report.speedup,
    )
}
