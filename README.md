# Blink Search Fuzzy Finder
This is a global search based on `fd` and `fzf` in often-visited locations.

Running `bl` gives you an interactive `fzf` window on your preferred location.

If you run `bl` the first time, your configuration is still empty.
To change this, run `bl -g` to get the location of the config file.
Here, you can specify your location. By default, the first location is shown:

```yml
locations:
  docs:
    path: /home/user/Documents
    mode: files
  local-nas-smb:
    path: \\\\nas.local\\share
    mode: folders
    cache_file: .blink\\all-folders.txt
```

Now, when you run `bl`, you can open any file from within `/home/user/Documents`.
To switch to another location, hit `[TAB]` and choose from the menu.
You can also use `[TAB]` again to accept the selection.

Alternatively, run `bl local-nas-smb` or `bl nas` for short to directly use the second location.

# Create Windows Installer MSI
First, install [WiX Toolset 3](https://github.com/wixtoolset/wix3/releases) ([Wix 3 Documentation](https://wixtoolset.org/docs/v3))

```ps
cargo install cargo-wix
$env:path += ";C:\Program Files (x86)\WiX Toolset v3.14\bin"

$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release

cargo wix --nocapture
```

## Recreate WiX config
Recreate WiX config based on `Cargo.toml`: (Will overwrite all Modifications!)
1. delete folder `\wix`
2. `cargo wix init`

# Similar Projects
- ff by genotrance: https://github.com/genotrance/ff
