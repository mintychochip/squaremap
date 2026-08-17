use squaremap_compare::{compare::{compare_output, compare_output_normalized}, recording::{decode, encode}, report::{validate_evidence, write_report}};
use squaremap_compare::evidence::validate_bundle;
use std::{env, fs, process::ExitCode, path::Path};
use squaremap_compare::parity::{ParityProbeManifest, run_manifest, write_parity_report};

fn main() -> ExitCode {
    match run() { Ok(equal) => if equal { ExitCode::SUCCESS } else { ExitCode::from(1) }, Err(error) => { eprintln!("{error}"); ExitCode::from(2) } }
}
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("parity") => {
            let manifest_path = flag_value(&args[1..], "--manifest")?;
            let java_root = flag_value(&args[1..], "--java-root")?;
            let rust_root = flag_value(&args[1..], "--rust-root")?;
            let report_path = flag_value(&args[1..], "--report")?;
            let manifest = ParityProbeManifest::load(&manifest_path)?;
            let report = run_manifest(&manifest, Path::new(&java_root), Path::new(&rust_root))?;
            write_parity_report(Path::new(&report_path), &report)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(report.complete && report.mismatches.is_empty())
        }
        Some("compare-output") => {
            let (java, rust, report_path, normalized) = compare_args(&args[1..])?;
            let report = if normalized {
                compare_output_normalized(java, rust)?
            } else {
                compare_output(java, rust)?
            };
            if let Some(path) = report_path { write_report(path, &report)?; }
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(report.mismatch_count == 0)
        }
        Some("replay") => {
            let path = flag_value(&args[1..], "--recording")?;
            if let Some(output) = flag_optional(&args[1..], "--output") {
                let recording = decode(&fs::read(path)?)?;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output)?;
                std::io::Write::write_all(&mut file, &encode(&recording))?;
                println!("copied {} frames", recording.frames.len());
                return Ok(true);
            }
            Err("replay requires an authenticated bridge sink; use --output for recording round-trip only".into())
        }
        Some("benchmark") => {
            let iterations = flag_value(&args[1..], "--iterations")?.parse::<u64>()?;
            if iterations == 0 { return Err("benchmark iterations must be greater than zero".into()); }
            let threshold = flag_value(&args[1..], "--threshold")?.parse::<f64>()?;
            if !threshold.is_finite() || threshold <= 0.0 { return Err("benchmark threshold must be finite and positive".into()); }
            let report = match flag_optional(&args[1..], "--java") {
                Some(java) => {
                    let rust = flag_value(&args[1..], "--rust")?;
                    squaremap_compare::benchmark::run_output(java, rust, iterations, threshold)?
                }
                None => squaremap_compare::benchmark::run(iterations, threshold),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(report.passed)
        }
        Some("report") => {
            let path = flag_value(&args[1..], "--input")?;
            let output = flag_value(&args[1..], "--output")?;
            let report: squaremap_compare::compare::ComparisonReport = serde_json::from_slice(&fs::read(path)?)?;
            write_report(output, &report)?;
            Ok(report.mismatch_count == 0)
        }
        Some("gate") => {
            let evidence = flag_value(&args[1..], "--evidence")?;
            validate_bundle(&evidence)?;
            let matrix = flag_optional(&args[1..], "--matrix").unwrap_or_else(|| "docs/superpowers/verification/rust-backend-parity-matrix.json".into());
            let commit = String::from_utf8(std::process::Command::new("git").args(["rev-parse", "HEAD"]).output()?.stdout)?.trim().to_owned();
            let dirty = !std::process::Command::new("git").args(["status", "--porcelain"]).output()?.stdout.is_empty();
            validate_evidence(evidence, matrix, &commit, dirty)?;
            println!("evidence gate passed");
            Ok(true)
        }
        _ => Err("usage: compare-output --java DIR --rust DIR [--report FILE] [--normalize-live] | replay --recording FILE [--output FILE] | benchmark --iterations N --threshold N [--java DIR --rust DIR] | report --input FILE --output FILE | gate --evidence FILE [--matrix FILE]".into())
    }
}

fn compare_args(args: &[String]) -> Result<(String, String, Option<String>, bool), Box<dyn std::error::Error>> {
    let java = flag_value(args, "--java")?;
    let rust = flag_value(args, "--rust")?;
    let report = flag_optional(args, "--report");
    let normalized = args.iter().any(|arg| arg == "--normalize-live");
    Ok((java, rust, report, normalized))
}

fn flag_value(args: &[String], flag: &str) -> Result<String, Box<dyn std::error::Error>> {
    let index = args.iter().position(|arg| arg == flag).ok_or_else(|| format!("missing {flag}"))?;
    args.get(index + 1).filter(|value| !value.starts_with("--")).cloned().ok_or_else(|| format!("missing value for {flag}").into())
}

fn flag_optional(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).filter(|value| !value.starts_with("--")).cloned()
}
