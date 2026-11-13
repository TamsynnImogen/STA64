//! The STA64 relay: `relay [--listen ADDR] [--max-rooms N] ...` (NET-03,
//! docs/NET.md section 9). Forwards lockstep frames between the members of a
//! room; it has no game content and decodes no message bodies.

use std::time::Duration;

use sta_net::transport::{Relay, RelayConfig};

fn main() {
    let mut cfg = RelayConfig::default();
    let mut listen = String::from("0.0.0.0:7864");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| -> String {
            args.next().unwrap_or_else(|| {
                eprintln!("relay: {name} needs a value");
                std::process::exit(2)
            })
        };
        let num = |name: &str, v: String| -> usize {
            v.parse().unwrap_or_else(|_| {
                eprintln!("relay: {name}: not a number: {v}");
                std::process::exit(2)
            })
        };
        match a.as_str() {
            "--listen" => listen = value("--listen"),
            "--max-connections" => cfg.max_connections = num(&a, value(&a)),
            "--max-connections-per-ip" => cfg.max_connections_per_ip = num(&a, value(&a)),
            "--max-rooms" => cfg.max_rooms = num(&a, value(&a)),
            "--max-rooms-per-ip" => cfg.max_rooms_per_ip = num(&a, value(&a)),
            "--idle-seconds" => cfg.idle_timeout = Duration::from_secs(num(&a, value(&a)) as u64),
            "--help" | "-h" => {
                println!(
                    "usage: relay [--listen ADDR:PORT] [--max-connections N] \
                     [--max-connections-per-ip N] [--max-rooms N] [--max-rooms-per-ip N] \
                     [--idle-seconds N]"
                );
                return;
            }
            other => {
                eprintln!("relay: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let relay = match Relay::bind(listen.as_str(), cfg) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("relay: cannot listen on {listen}: {e}");
            std::process::exit(1);
        }
    };
    println!("relay listening on {}", relay.local_addr());
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
