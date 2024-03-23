use anyhow::Result;
use regex::Regex;
use config::{Config, Location, LocationMode};
use memchr;
use std::{env, ffi::OsString, fs::{File, OpenOptions}, io::{self, BufRead, BufReader, Write}, path::{Path, PathBuf}, process::{exit, ChildStdout, Command, Stdio}, str::FromStr};
use clap::{Parser, ValueEnum};
use log::{info, debug};
use simplelog::{LevelFilter, WriteLogger};
use strum;
mod config;

fn open_folder(path: &str) -> Result<()> {
    let path = path.trim();
    debug!("open_folder({})", path);

    let path = path.replace("\\", "/");
    let path = Regex::new(r"/+").unwrap().replace_all(&path, "/");

    let mut cmd = if cfg!(target_os = "windows") {
        let mut path = path.to_string();
        if path.starts_with('/') { path = format!("/{}", path); }
        path = path.replace("/", "\\");
        path = path.trim_end_matches('\\').to_owned();
        let mut cmd = Command::new("explorer");
        cmd.arg(OsString::from_str(&path)?);
        cmd
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(OsString::from_str(&path)?);
        cmd
    };
    cmd
        .with(|b| debug!("Executing: {:?}", b))
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

fn normalize(file_names: Stdio, sep: Separator) -> Result<ChildStdout> {
    Ok(Command::new(env::current_exe()?)
        .arg(format!("--normalize-paths={}", sep))
        .stdin(file_names)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .with(|b| debug!("Executing: {:?}", b))
        .spawn()?
        .stdout.unwrap())
}

fn read_location_from_cache(path: PathBuf) -> Result<ChildStdout> {
    info!("Reading cache file: \"{}\"", path.to_string_lossy());
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => {
            info!("Cache file {} not found. Please check your configuration.", path.to_string_lossy());
            exit(-1);
        }
    };
    normalize(file.into(), Separator::Newline)
}

fn read_location_cmd(location: &Location, config: &Config) -> Command {
    let mut cmd = run("fd");
    cmd
        .arg(".")
        .arg("--print0")
        .arg("--type").arg(match location.mode {
            LocationMode::Files => "f",
            LocationMode::Folders => "d",
        })
        .args(config.fd_flags.as_ref().unwrap_or(&Vec::new()))
        .current_dir(&location.path)
        .with(|b| debug!("Executing: {:?}", b));
    cmd
}

fn read_location_with_fd(location: &Location, config: &Config) -> Result<ChildStdout> {
    let fd_list = read_location_cmd(location, config)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?
        .stdout.unwrap();
    normalize(fd_list.into(), Separator::Null)
}

enum OpenAction {
    Open(PathBuf),
    Menu
}
fn fzf_open(location_name: &str, location: &Location, config: &Config) -> Result<OpenAction> {
    let this_exe = env::current_exe()?;

    let fzf_input_list = match &location.cache_file {
        Some(cache_file) => read_location_from_cache(Path::new(&location.path).join(cache_file))?,
        None => read_location_with_fd(location, config)?,
    };

    let mut out = run("fzf")
        .arg("--scheme=path")
        .arg(format!("--history={}", Config::base_dir().join(format!("history-{}.txt", location_to_id(location_name)?)).to_string_lossy()))
        .arg("--bind=tab:execute(echo TAB)+abort")
        .arg(format!("--bind=ctrl-x:execute(\"{}\" --open-path={{}} {})", this_exe.display(), location_name))
        .arg("--bind=alt-c:execute(echo EDIT_CONFIG)+abort")
        .args(config.fzf_flags.as_ref().unwrap_or(&Vec::new()))

        .stdin(fzf_input_list)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .with(|b| debug!("Executing: {:?}", b))
        .spawn()?;

    let reader = std::io::BufReader::new(out.stdout.as_mut().unwrap());
    let mut action: Option<OpenAction> = None;
    for line in reader.lines() {
        debug!("Reading fzf output line: {:?}", line);
        assert!(action.is_none());
        action = match line {
            Ok(ref s) if s == "TAB" => Some(OpenAction::Menu),
            Ok(ref s) if s == "EDIT_CONFIG" => Some(OpenAction::Open(Config::path())),
            Ok(s) => {
                debug!("FZF output: \"{}\"", s);
                let s = match s.trim() {
                    s if s.starts_with('"') && s.ends_with('"') => s[1..s.len()-1].replace("\\\\", "\\"),
                    s => s.to_owned(),
                };
                Some(OpenAction::Open(Path::new(&location.path).join(s)))
            },
            Err(e) => panic!("Error reading line: {}", e),
        }
    }

    let status = out.wait()?;
    let ret = status.code().unwrap();
    match (ret, action) {
        (130, Some(OpenAction::Menu)) => Ok(OpenAction::Menu),
        (_, Some(OpenAction::Open(path))) => Ok(OpenAction::Open(path)),
        _ => return Err(anyhow::anyhow!("fzf exited with code {}", ret)),
    }
}

fn fzf_menu(query: Option<&str>, config: &Config) -> Result<String> {
    let fzf = run("fzf")
        .arg(format!("--history={}", Config::base_dir().join("history-menu.txt").to_string_lossy()))
        .arg("--bind").arg("tab:accept")
        .with(|b| if let Some(q) = query { b.arg(format!("--query={}", q)); })
        .args(config.fzf_flags.as_ref().unwrap_or(&Vec::new()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .with(|b| debug!("Executing: {:?}", b))                
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
    #[arg(long)]
    open_path: Option<String>,

    /// Normalizes all paths from stdin separated by NULL bytes to a native format separeted by newline. Useful for scripting.
    #[arg(long)]
    normalize_paths: Option<Separator>,

    /// Specify the location to search.
    /// 
    /// Accepts shortened if unique.
    /// If not specified, the first location in the config will be used.
    location: Option<String>,
}

#[derive(Parser, Clone, ValueEnum, strum::Display)]
enum Separator {
    #[strum(serialize = "null")]
    Null,
    #[strum(serialize = "newline")]
    Newline,
}

#[test]
fn verify_cli() {
    use clap::CommandFactory;
    Args::command().debug_assert()
}

fn main() -> Result<()> {
    let config = Config::new()?;

    let log_name = "blink.log";
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(Config::base_dir().join(log_name))?;
    WriteLogger::init(LevelFilter::Debug, simplelog::Config::default(), log_file)?;

    debug!("Command line: {:?}", std::env::args().collect::<Vec<String>>());

    let args = Args::parse();

    if let Some(separator) = args.normalize_paths {
        let separator = match separator {
            Separator::Null => b'\0',
            Separator::Newline => b'\n',
        };
        for line in BufReader::new(io::stdin()).split(separator) {
            let s: String = String::from_utf8_lossy(&line?)
                .trim()
                .trim_start_matches("./")
                .trim_start_matches(".\\")
                .chars().map(|c| if c.is_control() { char::REPLACEMENT_CHARACTER } else { c }).collect();
            println!("{}", Path::new(&s).to_string_lossy());
        }
        return Ok(());
    }

    if args.get_config_path {
        println!("{}", Config::path().to_string_lossy());
        return Ok(());
    }

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
        io::copy(&mut read_location_with_fd(loc, &config)?, &mut io::stdout())?;
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
        match fzf_open(&location_name, loc, &config)? {
            OpenAction::Open(path) => {
                let s = path.to_string_lossy();
                debug!("Opening: \"{}\"", s);
                open_folder(&s).unwrap();
                return Ok(());
            }, OpenAction::Menu => {
                location_name = fzf_menu(None, &config)?;
                info!("Selected location: {}", location_name);
            },
        }
    }
}

// Extend Command Builder with with() function
trait WithFunction {
    fn with<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut Self);
}
impl WithFunction for Command {
    fn with<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut Self)
    {
        f(self);
        self
    }
}

// Extend BufRead with split2() function
trait Split2Ext: BufRead + Sized {
    fn split2(self, delim1: u8, delim2: u8) -> Split2<Self>;
}
impl<R: BufRead> Split2Ext for R {
    fn split2(self, delim1: u8, delim2: u8) -> Split2<Self> {
        Split2 { reader: self, delim: (delim1, delim2) }
    }
}
struct Split2<R: BufRead> {
    reader: R,
    delim: (u8, u8),
}
impl<R: BufRead> Iterator for Split2<R> {
    type Item = Result<Vec<u8>>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut buf = Vec::new();
        loop {
            let available = match self.reader.fill_buf() {
                Ok(s) => s,
                Err(e) => return Some(Err(e.into())),
            };
            let (done, used) = match memchr::memchr2(self.delim.0, self.delim.1, available) {
                Some(i) => {
                    buf.extend_from_slice(&available[..=i]);
                    (true, i+1)
                },
                None => {
                    buf.extend_from_slice(available);
                    (false, available.len())
                },
            };
            self.reader.consume(used);
            if done || used == 0 {
                break;
            }
        }
        while buf.last() == Some(&self.delim.0) || buf.last() == Some(&self.delim.1) {
            buf.pop();
        }
        match buf.len() {
            0 => None,
            _ => Some(Ok(buf)),
        }
    }
}
