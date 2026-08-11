mod bootstrap;

use bootstrap::{parse_connect, read_token, run_bridge};
use std::env;
use std::io::{self, BufReader};

fn print_help() {
    println!("Usage: squaremap-server bridge --connect 127.0.0.1:<port> --plugin-version <version>");
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return;
    };
    if command == "--help" || command == "-h" {
        print_help();
        return;
    }
    if command != "bridge" {
        eprintln!("unknown command: {command}");
        std::process::exit(2);
    }

    let mut connect = None;
    let mut plugin_version = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--connect" => connect = args.next(),
            "--plugin-version" => plugin_version = args.next(),
            "--help" | "-h" => {
                print_help();
                return;
            }
            other => {
                eprintln!("unknown bridge argument: {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(connect) = connect else {
        eprintln!("bridge requires --connect");
        std::process::exit(2);
    };
    let Some(plugin_version) = plugin_version else {
        eprintln!("bridge requires --plugin-version");
        std::process::exit(2);
    };
    let connect = match parse_connect(&connect) {
        Ok(address) => address,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let token = match read_token(BufReader::new(io::stdin().lock())) {
        Ok(token) => token,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime should initialize");
    let _ = tracing_subscriber::fmt().with_target(false).try_init();
    if let Err(error) = runtime.block_on(run_bridge(connect, &plugin_version, token)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
