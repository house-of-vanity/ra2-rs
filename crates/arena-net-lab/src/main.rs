use arena_net_lab::runner;
fn main() {
    let mut out = std::path::PathBuf::from("logs/net-report");
    let mut seeds = vec![7u64, 19, 41];
    let mut only = None;
    let mut players = vec![2usize, 4];
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().expect("--out PATH").into(),
            "--seed" => {
                seeds = vec![
                    args.next()
                        .expect("--seed N")
                        .parse()
                        .expect("seed integer"),
                ]
            }
            "--profile" => only = args.next(),
            "--players" => {
                players = vec![
                    args.next()
                        .expect("--players N")
                        .parse()
                        .expect("players integer"),
                ]
            }
            "--help" => {
                println!("arena-net-lab [--out DIR] [--seed N] [--profile NAME] [--players 2|4]");
                return;
            }
            _ => panic!("unknown option {arg}"),
        }
    }
    std::fs::create_dir_all(&out).unwrap();
    let mut csv = runner::Report::csv_header().to_string();
    csv.push('\n');
    let mut failed = false;
    let mut count = 0;
    println!(
        "profile                 peers RTT  pred p95/max m    interp p95 m  host KiB/s  result"
    );
    for profile in runner::profiles()
        .into_iter()
        .filter(|p| only.as_ref().is_none_or(|s| s == &p.name))
    {
        for &n in &players {
            for &seed in &seeds {
                let report =
                    runner::run(profile.clone(), n, seed).expect("network experiment failed");
                count += 1;
                println!(
                    "{:<24} {} {:>4}  {:>6.3}/{:<6.3}   {:>6.3}       {:>7.2}   {}",
                    report.profile,
                    n,
                    report.rtt_ms,
                    report.correction_p95_m,
                    report.correction_max_m,
                    report.interpolation_p95_m,
                    report.host_kib_per_s,
                    if report.passed() { "PASS" } else { "FAIL" }
                );
                let mut trace = runner::Trace::csv_header().to_string();
                trace.push('\n');
                for sample in &report.trace {
                    trace.push_str(&sample.csv());
                    trace.push('\n');
                }
                std::fs::write(
                    out.join(format!("{}-{n}-{seed}.csv", report.profile)),
                    trace,
                )
                .unwrap();
                for error in &report.failures {
                    eprintln!("  {error}");
                }
                failed |= !report.passed();
                csv.push_str(&report.csv());
                csv.push('\n');
            }
        }
    }
    if count == 0 {
        eprintln!("no matching profiles");
        std::process::exit(2);
    }
    std::fs::write(out.join("metrics.csv"), csv).unwrap();
    println!(
        "{count} experiments; report {}",
        out.join("metrics.csv").display()
    );
    if failed {
        std::process::exit(1)
    }
}
