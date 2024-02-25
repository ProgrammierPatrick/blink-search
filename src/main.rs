use anyhow::Result;
use regex::Regex;
use config::{Config, Location, LocationMode};
use std::{fs::{File, OpenOptions}, io::{BufRead, Write}, path::Path, process::{exit, Command, Stdio}, env};
use clap::Parser;
use log::{info, debug};
use simplelog::{CombinedLogger, LevelFilter, TermLogger, TerminalMode, WriteLogger, ColorChoice};
mod config;

fn open_folder(path: &str) -> Result<()> {
    let path = path.trim();
    debug!("open_folder({})", path);

    let path = path.replace("\\", "/");
    let path = Regex::new(r"/+").unwrap().replace_all(&path, "/");

    let (exe, path) = if cfg!(target_os = "windows") {
        let mut path = path.to_string();
        if path.starts_with('/') { path = format!("/{}", path); }
        path = path.replace("/", "\\");
        ("explorer", path.replace("/", "\\"))
    } else {
        ("xdg-open", path.to_string())
    };

    info!("Executing {} {}", exe, path);

    Command::new(exe)
        .arg(path)
        .spawn()?;

    Ok(())
}

fn location_to_id(location: &str) -> Result<String> {
    let r = Regex::new(r"[^a-zA-Z0-9]").unwrap().replace_all(location, "");
    Ok(r.to_lowercase())
}

fn run(exe: &str) -> Command {
    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
    Command::new(format!("{}{}", exe, ext))
}

enum OpenAction { Open, Menu }
fn fzf_open(location_name: &str, location: &Location) -> Result<OpenAction> {
    let fd_list: Stdio = match &location.cache_file {
        Some(cache_file) => {
            let path = Path::new(&location.path).join(cache_file);
            info!("Reading cache file: {}", path.to_string_lossy());
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => {
                    info!("Cache file {} not found. Please check your configuration.", path.to_string_lossy());
                    exit(-1);
                }
            };
            file.into()
        },
        None => {
            run("fd")
                .arg(".")
                .arg("--type").arg(match location.mode {
                    LocationMode::Files => "f",
                    LocationMode::Folders => "d",
                })

                .current_dir(&location.path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())

                .spawn()?
                .stdout
                .unwrap()
                .into()
        }
    };

    let mut out = run("fzf")
        .arg("--scheme=path")
        .arg(format!("--history={}", Config::base_dir().join(format!("history-{}.txt", location_to_id(location_name)?)).to_string_lossy()))
        .arg("--bind").arg("tab:execute(echo TAB)+abort")
        .arg("--bind").arg(format!("ctrl-x:execute({} --open-path={{}} {})", env::current_exe()?.to_string_lossy(), location_name))

        .stdin(fd_list)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let reader = std::io::BufReader::new(out.stdout.as_mut().unwrap());
    let mut is_tab = false;
    for line in reader.lines() {
        match line {
            Ok(ref s) if s == "TAB" => is_tab = true,
            Ok(s) => {
                debug!("FZF output: {}", s);
                let s = match s.trim() {
                    s if s.starts_with('"') && s.ends_with('"') => s[1..s.len()-1].replace("\\\\", "\\"),
                    s => s.to_owned(),
                };
                debug!("Opening: {}", s);
                open_folder(&Path::new(&location.path).join(s).to_string_lossy()).unwrap()
            },
            Err(e) => panic!("Error reading line: {}", e),
        }
    }

    let status = out.wait()?;
    let ret = status.code().unwrap();

    match (ret, is_tab) {
        (130, true) => return Ok(OpenAction::Menu),
        (0, false) => Ok(OpenAction::Open),
        _ => return Err(anyhow::anyhow!("fzf exited with code {}, is_tab={}", ret, is_tab)),
    }
}

fn fzf_menu(query: Option<&str>, config: &Config) -> Result<String> {
    let mut fzf = run("fzf");
    fzf.arg(format!("--history={}", Config::base_dir().join("history-menu.txt").to_string_lossy()));
    fzf.arg("--bind").arg("tab:accept");
    if let Some(q) = query {
        fzf.arg(format!("--query={}", q));
    }

    let fzf = fzf.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    for s in config.locations.iter().map(|(name, loc)| format!("{} ({})", name, loc.path)) {
        writeln!(fzf.stdin.as_ref().unwrap(), "{}", s)?;
    }

    let out = fzf.wait_with_output()?;
    let ret = out.status.code().unwrap();
    let str = String::from_utf8_lossy(&out.stdout);
    match (ret, str.as_ref()) {
        (0, s) => {
            let selection = config.locations.iter()
                .map(|(name, loc)| (name, format!("{} ({})", name, loc.path)))
                .find(|(_, text)| text == s.trim())
                .map(|(name, _)| name.to_owned()).unwrap();
            Ok(selection)
        }
        _ => Err(anyhow::anyhow!("fzf exited with code {}", ret)),
    }
}

#[derive(Parser)]
#[command(name="blink search", version, about)]
struct Args {

    /// Writes all files or folders to stdout. Useful for automating cache creation.
    #[arg(short, long)]
    create_cache: bool,

    /// List all available locations.
    #[arg(short, long)]
    list_locations: bool,

    /// Print the config path.
    #[arg(short, long)]
    get_config_path: bool,

    /// Directly open path using this query. Useful for scripting.
    #[arg(short, long)]
    open_path: Option<String>,

    /// Specify the location to search.
    /// 
    /// Accepts shortened if unique.
    /// If not specified, the first location in the config will be used.
    location: Option<String>,
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Args::command().debug_assert()
}

fn main() -> Result<()> {
    let args = Args::parse();

    let log_name = if args.open_path.is_some() { "blink-open.log" } else { "blink.log" };
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(Config::base_dir().join(log_name))?;
    WriteLogger::init(LevelFilter::Debug, simplelog::Config::default(), log_file)?;

    if args.get_config_path {
        println!("{}", Config::path().to_string_lossy());
        return Ok(());
    }

    let config = Config::new()?;

    if args.list_locations {
        for (name, loc) in config.locations.iter() {
            println!("{} ({})", name, loc.path);
        }
        return Ok(());
    }

    if config.locations.is_empty() {
        println!("No locations defined");
        println!("Define locations in {}", Config::path().to_string_lossy());
        println!("Example config with some locations:");
        println!("locations:");
        println!("  home:");
        println!("    path: /home/user");
        println!("    mode: files");
        println!("  nas:");
        println!("    path: \\\\nas.local\\share");
        println!("    mode: folders");
        println!("    cache_file: .blink\\all-folders.txt");
        println!();

        return Ok(());
    }

    let mut location_name: String = match args.location {
        None => config.locations.keys().next().unwrap().to_owned(),
        Some(loc) => {
            if config.locations.contains_key(&loc) {
                loc
            } else {
                let mut matches = config.locations.keys()
                    .filter(|k| k.to_lowercase().contains(&loc.to_lowercase()));
                match (matches.next(), matches.next()) {
                    (Some(_), Some(_)) => fzf_menu(Some(&loc), &config)?,
                    (Some(name), None) => name.to_owned(),
                    (None, None) => return Err(anyhow::anyhow!("No location found")),
                    _ => return Err(anyhow::anyhow!("logic error")),
                }
            }
        },
    };

    if args.create_cache {
        debug!("Creating cache for {}", location_name);
        let loc = config.locations.get(&location_name).unwrap();
        run("fd")
            .arg(".")
            .arg("--type").arg(match loc.mode {
                LocationMode::Files => "f",
                LocationMode::Folders => "d",
            })
            .current_dir(&loc.path)
            .spawn()?
            .wait()?;
        return Ok(());
    }

    match args.open_path {
        Some(ref s) => {
            debug!("execute --open-path={} with location {}", s, location_name);
            let loc = config.locations.get(&location_name).unwrap();
            open_folder(&Path::new(&loc.path).join(s).to_string_lossy()).unwrap();
            return Ok(());
        },
        None => (),
    }

    loop {
        let loc = config.locations.get(&location_name).unwrap();
        match fzf_open(&location_name, loc)? {
            OpenAction::Open => return Ok(()),
            OpenAction::Menu => {
                location_name = fzf_menu(None, &config)?;
                info!("Selected location: {}", location_name);
            },
        }
    }
}
