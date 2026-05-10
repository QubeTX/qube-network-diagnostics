use clap::CommandFactory;
use std::fs;

// Include the cli module types for man page generation.
// We reference the library's cli module via the crate path.
#[path = "src/cli.rs"]
mod cli;

// Stub the dependency so cli.rs compiles in the build script context.
// cli.rs uses `crate::speedtest::TestDuration` — we replicate just enough here.
mod speedtest {
    #[derive(Clone)]
    #[allow(dead_code)]
    pub enum TestDuration {
        Seconds(u64),
        Auto,
    }
}

fn normalize_manpage(buf: Vec<u8>) -> Vec<u8> {
    let rendered = String::from_utf8(buf).expect("clap_mangen renders UTF-8 man pages");
    rendered
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn main() {
    let man_dir =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap_or_else(|_| "man".to_string()))
            .join("man");
    fs::create_dir_all(&man_dir).unwrap();

    // Also write to project root man/ for inclusion in release archives
    let root_man_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("man");
    fs::create_dir_all(&root_man_dir).unwrap();

    // nd300
    let nd300_cmd = cli::Nd300Cli::command();
    let nd300_man = clap_mangen::Man::new(nd300_cmd);
    let mut buf = Vec::new();
    nd300_man.render(&mut buf).unwrap();
    let buf = normalize_manpage(buf);
    fs::write(man_dir.join("nd300.1"), &buf).unwrap();
    fs::write(root_man_dir.join("nd300.1"), &buf).unwrap();

    // speedqx
    let speedqx_cmd = cli::SpeedQXCli::command();
    let speedqx_man = clap_mangen::Man::new(speedqx_cmd);
    let mut buf = Vec::new();
    speedqx_man.render(&mut buf).unwrap();
    let buf = normalize_manpage(buf);
    fs::write(man_dir.join("speedqx.1"), &buf).unwrap();
    fs::write(root_man_dir.join("speedqx.1"), &buf).unwrap();

    println!("cargo::rerun-if-changed=src/cli.rs");
}
