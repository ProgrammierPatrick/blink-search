use anyhow::Result;
use regex::Regex;
use config::{Config, Location, LocationMode};
use std::{fs::File, io::Write, path::Path, process::{exit, Command, Stdio}};
use clap::Parser;
mod config;

fn open_folder(path: &str) -> Result<()> {
    let (exe, path) = if cfg!(target_os = "windows") {
        ("explorer", path.replace("/", "\\"))
    } else {
        ("xdg-open", path.to_owned())
    };

    println!("Executing {} {}", exe, path);
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

enum OpenAction { Open(String), Menu }
fn fzf_open(location_name: &str, location: &Location) -> Result<OpenAction> {
    let fd_list: Stdio = match &location.cache_file {
        Some(cache_file) => {
            let path = Path::new(&location.path).join(cache_file);
            println!("Reading cache file: {}", path.to_string_lossy());
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => {
                    println!("Cache file {} not found. Please check your configuration.", path.to_string_lossy());
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

    let out = run("fzf")
        .arg("--scheme=path")
        .arg(format!("--history={}", Config::base_dir().join(format!("history-{}.txt", location_to_id(location_name)?)).to_string_lossy()))
        .arg("--bind").arg("tab:execute(echo TAB)+abort")

        .stdin(fd_list)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?
        .wait_with_output()?;

    let ret = out.status.code().unwrap();
    let str = String::from_utf8_lossy(&out.stdout);

    println!("Exit code: {}, output: '{}'", ret, str.as_ref());

    match (ret, str.as_ref().trim()) {
        (130, "TAB") => return Ok(OpenAction::Menu),
        (0, s) => Ok(OpenAction::Open(s.trim().to_string())),
        _ => return Err(anyhow::anyhow!("fzf exited with code {}", ret)),
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

    loop {
        let loc = config.locations.get(&location_name).unwrap();
        match fzf_open(&location_name, loc)? {
            OpenAction::Open(path) => {
                open_folder(&Path::new(&loc.path).join(path).to_string_lossy())?;
                return Ok(())
            },
            OpenAction::Menu => {
                location_name = fzf_menu(None, &config)?;
                println!("Selected location: {}", location_name);
            },
        }
    }
    
}
