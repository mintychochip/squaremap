use squaremap_compare::{compare::compare_output, recording::decode, report::write_report};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    match run() { Ok(equal) => if equal { ExitCode::SUCCESS } else { ExitCode::from(1) }, Err(error) => { eprintln!("{error}"); ExitCode::from(2) } }
}
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("compare-output") => {
            let java = required(&mut args, "--java")?;
            let rust = required(&mut args, "--rust")?;
            let report_path = optional(&mut args, "--report");
            let report = compare_output(java, rust)?;
            if let Some(path) = report_path { write_report(path, &report)?; }
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(report.mismatch_count == 0)
        }
        Some("replay") => {
            let path = required(&mut args, "--recording")?;
            let recording = decode(&fs::read(path)?)?;
            println!("replayed {} frames", recording.frames.len());
            Ok(true)
        }
        Some("benchmark") => {
            let iterations = optional(&mut args, "--iterations")
                .ok_or("benchmark requires --iterations")?
                .parse::<u64>()?;
            let threshold = optional(&mut args, "--threshold")
                .ok_or("benchmark requires --threshold")?
                .parse::<f64>()?;
            if threshold <= 0.0 {
                return Err("benchmark threshold must be positive".into());
            }
            let report = match optional(&mut args, "--java") {
                Some(java) => {
                    let rust = required(&mut args, "--rust")?;
                    squaremap_compare::benchmark::run_output(java, rust, iterations, threshold)?
                }
                None => squaremap_compare::benchmark::run(iterations, threshold),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(report.passed)
        }
        Some("report") => {
            let path = required(&mut args, "--input")?;
            let output = required(&mut args, "--output")?;
            let report: squaremap_compare::compare::ComparisonReport = serde_json::from_slice(&fs::read(path)?)?;
            write_report(output, &report)?;
            Ok(report.mismatch_count == 0)
        }
        _ => Err("usage: compare-output --java DIR --rust DIR [--report FILE] | replay --recording FILE | benchmark [--iterations N] [--threshold N] | report --input FILE --output FILE".into())
    }
}
fn required<I: Iterator<Item=String>>(args: &mut I, flag: &str) -> Result<String, Box<dyn std::error::Error>> { match args.next().as_deref() { Some(value) if value == flag => args.next().ok_or_else(|| format!("missing value for {flag}").into()), _ => Err(format!("expected {flag}").into()) } }
fn optional<I: Iterator<Item=String>>(args: &mut I, flag: &str) -> Option<String> { match args.next().as_deref() { Some(value) if value == flag => args.next(), _ => None } }
