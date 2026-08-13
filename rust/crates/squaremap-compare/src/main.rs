use squaremap_compare::{compare::{compare_output, compare_output_normalized}, recording::decode, report::write_report};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    match run() { Ok(equal) => if equal { ExitCode::SUCCESS } else { ExitCode::from(1) }, Err(error) => { eprintln!("{error}"); ExitCode::from(2) } }
}
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
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
            let recording = decode(&fs::read(path)?)?;
            println!("replayed {} frames", recording.frames.len());
            Ok(true)
        }
        Some("benchmark") => {
            let iterations = flag_value(&args[1..], "--iterations")?.parse::<u64>()?;
            let threshold = flag_value(&args[1..], "--threshold")?.parse::<f64>()?;
            if threshold <= 0.0 { return Err("benchmark threshold must be positive".into()); }
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
        _ => Err("usage: compare-output --java DIR --rust DIR [--report FILE] [--normalize-live] | replay --recording FILE | benchmark --iterations N --threshold N [--java DIR --rust DIR] | report --input FILE --output FILE".into())
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
