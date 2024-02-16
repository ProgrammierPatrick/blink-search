use std::{
    env::var,
    fs::File,
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::Result;
use zip::read::ZipArchive;

fn download_from_zip(url: &str, file_name: &str, out_dir: &Path) -> Result<()> {
    // check if out_dir already contains the file
    if out_dir.join(file_name).exists() {
        println!(
            "cargo:warning={} already exists in {}",
            file_name,
            out_dir.to_string_lossy()
        );
        return Ok(());
    }

    println!("cargo:warning=Downloading {}...", url);
    let response = reqwest::blocking::get(url)?;
    let zip_file = Cursor::new(response.bytes()?);
    println!("cargo:warning=Extracting {}...", file_name);
    let mut zip = ZipArchive::new(zip_file)?;
    let zip_file_path: String = zip
        .file_names()
        .find(|name| name.ends_with(file_name))
        .ok_or_else(|| anyhow::anyhow!("{} not found in zip", file_name))?
        .into();
    let mut file = zip.by_name(&zip_file_path)?;
    let out_path = out_dir.join(file_name);
    println!("cargo:warning=Writing to {}...", out_path.to_string_lossy());
    let mut out_file = File::create(&out_path)?;
    std::io::copy(&mut file, &mut out_file)?;

    Ok(())
}

// src: https://stackoverflow.com/a/67516503/13565664
fn get_output_path() -> PathBuf {
    //<root or manifest path>/target/<profile>/
    let manifest_dir_string = var("CARGO_MANIFEST_DIR").unwrap();
    let build_type = var("PROFILE").unwrap();
    Path::new(&manifest_dir_string)
        .join("target")
        .join(build_type)
        .into()
}

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=build.rs");

    // only run on windows
    if !cfg!(target_os = "windows") {
        return Ok(());
    }

    let out_dir = get_output_path();
    let out_dir = Path::new(&out_dir);

    let fd_url = "https://github.com/sharkdp/fd/releases/download/v9.0.0/fd-v9.0.0-x86_64-pc-windows-msvc.zip";
    let fzf_url = "https://github.com/junegunn/fzf/releases/download/0.46.1/fzf-0.46.1-windows_amd64.zip";

    download_from_zip(fd_url, "fd.exe", out_dir)?;
    download_from_zip(fzf_url, "fzf.exe", out_dir)?;

    Ok(())
}
