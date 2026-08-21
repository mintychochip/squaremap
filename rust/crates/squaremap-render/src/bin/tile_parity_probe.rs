#[allow(dead_code)]
#[path = "../../tests/support/tile_cases.rs"]
mod tile_cases;

use std::path::PathBuf;
use std::{env, fs, process};
use tile_cases::{load_manifest_result, probe_catalog};

struct Args {
    manifest: PathBuf,
    out: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async_main(args))
}

async fn async_main(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let (manifest, bytes) = load_manifest_result(&args.manifest).map_err(std::io::Error::other)?;
    let probe = probe_catalog(&manifest, &bytes).await?;
    let json = serde_json::to_string_pretty(&probe.report)?;
    if let Some(out) = &args.out {
        fs::create_dir_all(out)?;
        fs::write(out.join("report.json"), format!("{json}\n"))?;
        for case in &probe.tiles {
            for (relative, rgba) in case.paths.iter().zip(&case.rgba) {
                let dest = out.join(&case.id).join(rgba_relative(relative));
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(dest, rgba)?;
            }
        }
    }
    println!("{json}");
    Ok(())
}

fn rgba_relative(png_relative: &str) -> String {
    png_relative
        .strip_suffix(".png")
        .map(|stem| format!("{stem}.rgba"))
        .unwrap_or_else(|| format!("{png_relative}.rgba"))
}

fn parse_args() -> Result<Args, String> {
    let mut manifest = None;
    let mut out = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest" => {
                let value = args.next().ok_or("--manifest requires a path")?;
                manifest = Some(PathBuf::from(value));
            }
            "--out" => {
                let value = args.next().ok_or("--out requires a path")?;
                out = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        manifest: manifest.ok_or("missing --manifest")?,
        out,
    })
}
