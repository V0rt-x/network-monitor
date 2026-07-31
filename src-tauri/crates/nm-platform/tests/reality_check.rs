//! Measurement-model reality check.
//!
//! The product's whole design rests on one assumption: that sending our own ICMP echoes
//! to the endpoints an application talks to yields a usable signal, and that where it
//! does not, a TTL-limited path probe still tells the user *where* the path dies. This
//! test measures whether that is true against the server pools the product cares about.
//!
//! Run it deliberately and read the report:
//!
//! ```text
//! cargo test -p nm-platform --features network-tests --test reality_check -- --nocapture
//! ```
//!
//! It asserts almost nothing on purpose. The controls must answer — if they do not, the
//! machine running it has no usable connection and the numbers mean nothing — but the
//! game pools are the *subject* of the measurement, not something to assert about. Their
//! results belong in `docs/measurement-reality-check.md`.

#![cfg(all(windows, feature = "network-tests"))]

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

use nm_platform::icmp::{windows::WindowsIcmpProber, EchoOutcome, EchoRequest, IcmpProber};

const TIMEOUT: Duration = Duration::from_secs(1);
const PROBES_PER_POOL: u32 = 4;
const MAX_HOPS: u8 = 20;

/// One thing worth knowing the reachability of.
struct Pool {
    /// What it is, for the report.
    label: &'static str,
    /// Hostname to resolve, or a literal address.
    host: &'static str,
    /// Controls prove the measurement itself works; failures here invalidate the run.
    control: bool,
}

const POOLS: &[Pool] = &[
    Pool {
        label: "Cloudflare DNS (control)",
        host: "1.1.1.1",
        control: true,
    },
    Pool {
        label: "Google DNS (control)",
        host: "8.8.8.8",
        control: true,
    },
    // Valve's Steam Datagram Relay points of presence: the reference pool for Source
    // games, and published by Valve as ping endpoints.
    Pool {
        label: "Valve SDR ams",
        host: "ams.valve.net",
        control: false,
    },
    Pool {
        label: "Valve SDR fra",
        host: "fra.valve.net",
        control: false,
    },
    Pool {
        label: "Valve SDR sto",
        host: "sto.valve.net",
        control: false,
    },
    Pool {
        label: "Steam community",
        host: "steamcommunity.com",
        control: false,
    },
    Pool {
        label: "Steam API",
        host: "api.steampowered.com",
        control: false,
    },
    Pool {
        label: "Discord web",
        host: "discord.com",
        control: false,
    },
    Pool {
        label: "Discord gateway",
        host: "gateway.discord.gg",
        control: false,
    },
    Pool {
        label: "Riot",
        host: "riotgames.com",
        control: false,
    },
    Pool {
        label: "Riot auth",
        host: "auth.riotgames.com",
        control: false,
    },
    Pool {
        label: "EA",
        host: "ea.com",
        control: false,
    },
    Pool {
        label: "Epic Games",
        host: "epicgames.com",
        control: false,
    },
    Pool {
        label: "AWS eu-central-1",
        host: "ec2.eu-central-1.amazonaws.com",
        control: false,
    },
    Pool {
        label: "Google Cloud",
        host: "cloud.google.com",
        control: false,
    },
];

/// What repeated echoes to one address produced.
#[derive(Debug, Default)]
struct Summary {
    replies: u32,
    timeouts: u32,
    unreachable: u32,
    errors: u32,
    rtts_us: Vec<u128>,
}

impl Summary {
    fn median_ms(&self) -> Option<f64> {
        if self.rtts_us.is_empty() {
            return None;
        }
        let mut sorted = self.rtts_us.clone();
        sorted.sort_unstable();
        #[allow(clippy::cast_precision_loss)]
        sorted
            .get(sorted.len() / 2)
            .map(|micros| *micros as f64 / 1_000.0)
    }

    fn verdict(&self) -> &'static str {
        if self.replies > 0 {
            "ICMP works"
        } else if self.unreachable > 0 {
            "unreachable"
        } else if self.errors > 0 {
            "local failure"
        } else {
            "ICMP silent"
        }
    }
}

fn resolve(host: &str) -> Option<Ipv4Addr> {
    if let Ok(literal) = host.parse::<Ipv4Addr>() {
        return Some(literal);
    }
    (host, 0_u16).to_socket_addrs().ok()?.find_map(|addr| {
        match addr.ip() {
            IpAddr::V4(v4) => Some(v4),
            // IPv6-only results are skipped: the prober speaks IPv4 for now, and this
            // report is about what we can actually measure today.
            IpAddr::V6(_) => None,
        }
    })
}

fn measure(prober: WindowsIcmpProber, target: Ipv4Addr) -> Summary {
    let mut summary = Summary::default();
    for _ in 0..PROBES_PER_POOL {
        let request = EchoRequest::to(IpAddr::V4(target), TIMEOUT);
        match prober.echo(&request) {
            Ok(EchoOutcome::Replied { rtt, .. }) => {
                summary.replies += 1;
                summary.rtts_us.push(rtt.as_micros());
            }
            Ok(EchoOutcome::TimedOut) => summary.timeouts += 1,
            Ok(EchoOutcome::Unreachable { .. }) => summary.unreachable += 1,
            // A TTL expiry without asking for one would be surprising; count it as a
            // failure to measure rather than folding it into a category it is not.
            Ok(EchoOutcome::TtlExpired { .. }) | Err(_) => summary.errors += 1,
        }
    }
    summary
}

/// Walks outward until the destination answers or the hops run out.
///
/// Returns the last hop that identified itself and how far the walk got. This is the
/// fallback of last resort: when a target is silent, knowing that the path dies inside
/// the ISP is a different verdict from knowing it dies at the destination's edge.
fn walk_path(prober: WindowsIcmpProber, target: Ipv4Addr) -> (Option<Ipv4Addr>, u8, bool) {
    let mut last_named = None;
    let mut reached = false;
    let mut depth = 0;

    for ttl in 1..=MAX_HOPS {
        let request = EchoRequest::to(IpAddr::V4(target), TIMEOUT).with_ttl(ttl);
        match prober.echo(&request) {
            Ok(EchoOutcome::Replied { .. }) => {
                reached = true;
                depth = ttl;
                break;
            }
            Ok(EchoOutcome::TtlExpired {
                from: Some(IpAddr::V4(hop)),
                ..
            }) => {
                last_named = Some(hop);
                depth = ttl;
            }
            _ => {}
        }
    }

    (last_named, depth, reached)
}

#[test]
fn reports_how_the_real_world_answers_probes() {
    let prober = WindowsIcmpProber;
    let mut controls_answered = 0;
    let mut icmp_usable = 0;
    let mut measured = 0;

    println!(
        "\n{:<28} {:<16} {:>6} {:>10}  verdict",
        "pool", "address", "reply", "median"
    );
    println!("{}", "-".repeat(88));

    for pool in POOLS {
        let Some(address) = resolve(pool.host) else {
            println!(
                "{:<28} {:<16} {:>6} {:>10}  no IPv4 address",
                pool.label, "-", "-", "-"
            );
            continue;
        };

        let summary = measure(prober, address);
        measured += 1;
        let median = summary
            .median_ms()
            .map_or_else(|| "-".to_owned(), |ms| format!("{ms:.2} ms"));

        println!(
            "{:<28} {:<16} {:>3}/{:<2} {:>10}  {}",
            pool.label,
            address.to_string(),
            summary.replies,
            PROBES_PER_POOL,
            median,
            summary.verdict()
        );

        if summary.replies > 0 {
            icmp_usable += 1;
            if pool.control {
                controls_answered += 1;
            }
        } else {
            // The interesting case: can a path probe still say something useful?
            let (last_hop, depth, reached) = walk_path(prober, address);
            match last_hop {
                Some(hop) if !reached => {
                    println!("    path probe: dies after {hop} at hop {depth} of {MAX_HOPS}");
                }
                Some(hop) => println!("    path probe: reached target, last named hop {hop}"),
                None => println!("    path probe: no hop identified itself"),
            }
        }
    }

    println!("\n{icmp_usable} of {measured} resolved pools answered ICMP directly.");

    assert!(
        controls_answered > 0,
        "no control target answered; this machine has no usable connection, \
         so nothing in the report above means anything"
    );
}
