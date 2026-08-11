mod bootstrap;
mod session;

use squaremap_server::http::{HttpConfig, HttpServer};
use squaremap_server::output::OutputRoot;
use bootstrap::{parse_connect, read_token, run_bridge};
use std::env;
use std::io::{self, BufReader};
use std::net::SocketAddr;

fn print_help() {
    println!("Usage: squaremap-server bridge --connect 127.0.0.1:<port> --plugin-version <version>");
    println!("       squaremap-server serve-fixture --root <path> --bind <loopback:port>");
}
async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return;
    };
    if command == "serve-fixture" {
        let mut root = None;
        let mut bind = None;
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--root" => root = args.next(),
                "--bind" => bind = args.next(),
                "--help" | "-h" => { print_help(); return; }
                other => { eprintln!("unknown serve-fixture argument: {other}"); std::process::exit(2); }
            }
        }
        let Some(root) = root else { eprintln!("serve-fixture requires --root"); std::process::exit(2); };
        let Some(bind) = bind else { eprintln!("serve-fixture requires --bind"); std::process::exit(2); };
        let bind: SocketAddr = match bind.parse() {
            Ok(bind) => bind,
            Err(error) => { eprintln!("invalid bind address: {error}"); std::process::exit(2); }
        };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime should initialize");
        let result = runtime.block_on(async move {
            let output = OutputRoot::new(root)?;
            let mut server = HttpServer::bind(HttpConfig { bind, enabled: true, dev_frontend: None }, output).await?;
            println!("READY http_addr={}", server.local_addr().expect("enabled server has address"));
            wait_for_shutdown().await;
            server.shutdown().await
        });
        if let Err(error) = result { eprintln!("{error}"); std::process::exit(1); }
        return;
    }
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
