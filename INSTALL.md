# Installing markdoc-pdf

`markdoc-pdf` is a single binary. It has no runtime dependencies
beyond the OS and **fonts** — no Java, no Python, no Pandoc.

> **Fonts are a hard runtime requirement.** Nothing is embedded in
> the binary. `markdoc-pdf` asks for font *families* by name ("Noto
> Sans", "Noto Sans Mono", …) and resolves them against whatever is
> installed on the machine. With no fonts installed it panics with
> `NoMatch` from `fontique` before writing anything. Install Noto
> Sans — `apk add font-noto`, `apt install fonts-noto-core`, or
> `dnf install google-noto-sans-fonts` — and every Latin-script
> language renders. Windows and macOS already ship families that are
> in the fallback chain (Segoe UI / Arial, Helvetica), so this only
> bites on minimal Linux images and containers. Custom and branded
> fonts are loadable via `font_paths` in the style file.

## Option A: install from source with Cargo (Linux / macOS)

The easiest path if you already have Rust installed.

**Prerequisites**: Rust 1.93 or newer. If you don't have Rust:

```sh
# Linux / macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Cargo fetches the dependencies, so this repository is the only checkout:

```sh
git clone https://github.com/markdoc-rs/markdoc-pdf.git
cd markdoc-pdf
cargo install --path . --locked
```

This builds the release binary and copies it into `~/.cargo/bin/`.
Make sure that's on your `$PATH`:

```sh
echo $PATH | tr ':' '\n' | grep cargo
# Should print something like: /home/you/.cargo/bin
```

Verify:

```sh
markdoc-pdf --version
```

## Option B: build the binary and copy it manually (Linux / macOS)

Useful when you want to drop the binary into `/usr/local/bin/` or
hand it to someone without giving them the source tree.

```sh
cd markdoc-pdf
cargo build --release --bin markdoc-pdf
sudo cp target/release/markdoc-pdf /usr/local/bin/
markdoc-pdf --version
```

The release binary is a single ~25 MB executable. Strip it for
distribution:

```sh
strip target/release/markdoc-pdf       # ~12-15 MB after strip
```

## Option C: Windows

Two paths — pick whichever fits your shell.

### C1. PowerShell + rustup

1. Install **Rust** via the official installer:
   <https://www.rust-lang.org/tools/install>. Pick the **MSVC**
   toolchain when prompted (default). The installer also installs
   the Visual Studio Build Tools if you don't already have them.
2. Clone the repository and install from it:

   ```powershell
   git clone https://github.com/markdoc-rs/markdoc-pdf.git
   cd markdoc-pdf
   cargo install --path . --locked
   ```
3. The binary lands at `%USERPROFILE%\.cargo\bin\markdoc-pdf.exe` —
   already on PATH thanks to the rustup installer. Verify:

   ```powershell
   markdoc-pdf --version
   ```

### C2. WSL (Windows Subsystem for Linux)

If you're already comfortable with WSL2 (Ubuntu, Debian, etc.), the
**Linux/macOS instructions in Option A apply unchanged**. The
binary lives inside the WSL filesystem and is only visible from
WSL — fine if you also edit your `.mdoc` files in WSL or via VS Code's
Remote-WSL.

To use a WSL-built binary from Windows-side tooling, copy it across:

```sh
# from inside WSL
cp ~/.cargo/bin/markdoc-pdf /mnt/c/Users/$USER/bin/
```

Then add `C:\Users\<you>\bin\` to your Windows PATH.

### Windows-specific notes

- Use `\` or `/` in path arguments — both work.
- The `--input` and `--output` flags accept absolute Windows paths
  (`C:\Users\you\Docs\manual.mdoc`); quote them if they contain spaces.
- File paths in your `.mdoc` (e.g. `![alt](images/photo.png)`) should
  use forward slashes — they're URI-style references resolved by
  the asset resolver, not OS paths.

## Option D: build inside a container

Useful when you don't want a Rust toolchain on your host, or when
your CI hands a writer a containerised build environment. Works
identically on Linux (Podman or Docker), macOS (Docker Desktop /
Podman Machine / OrbStack), and Windows (Docker Desktop / Podman
Machine).

Rust on Alpine links the binary statically, so it runs on any Linux
afterwards. That takes in fontconfig, the C library that finds the
fonts, and what it links in turn: the `*-static` packages below carry
their static archives, and `PKG_CONFIG_ALL_STATIC=1` has the build use
them.

### One-shot build

From the folder that holds your `markdoc-pdf/` checkout:

```sh
podman run --rm -v "$PWD":/work -w /work/markdoc-pdf -e PKG_CONFIG_ALL_STATIC=1 \
    docker.io/library/rust:1.93-alpine \
    sh -c 'apk add --no-cache musl-dev pkgconf fontconfig-dev fontconfig-static \
               freetype-static expat-static libpng-static zlib-static \
               brotli-static bzip2-static \
           && cargo build --release --bin markdoc-pdf'
# Binary lands at: markdoc-pdf/target/release/markdoc-pdf
```

(Replace `podman` with `docker` if that's what you have.)

### Reusable dev container

Build once, then drop into a shell with the toolchain ready:

```sh
podman build -t markdoc-pdf-dev - <<'EOF'
FROM docker.io/library/rust:1.93-alpine
RUN apk add --no-cache musl-dev pkgconf fontconfig-dev fontconfig-static \
        freetype-static expat-static libpng-static zlib-static \
        brotli-static bzip2-static git
ENV PKG_CONFIG_ALL_STATIC=1
WORKDIR /work
EOF

# From the folder that holds your markdoc-pdf/ checkout:
podman run --rm -it -v "$PWD":/work -w /work/markdoc-pdf \
    markdoc-pdf-dev sh
# inside the container:
cargo build --release --bin markdoc-pdf
exit
```

### Render docs without a host install

Run `markdoc-pdf` itself inside the container — useful for one-off
renders on a machine where you don't want to install anything. The
container has no fonts of its own, hence `font-noto`:

```sh
podman run --rm -v "$PWD":/work -w /work -e PKG_CONFIG_ALL_STATIC=1 \
    docker.io/library/rust:1.93-alpine \
    sh -c 'apk add --no-cache musl-dev pkgconf fontconfig-dev fontconfig-static \
               freetype-static expat-static libpng-static zlib-static \
               brotli-static bzip2-static font-noto >/dev/null \
           && cd markdoc-pdf && cargo build --release --bin markdoc-pdf 2>/dev/null \
           && ./target/release/markdoc-pdf -i ../my-doc/intro.mdoc -o /work/intro.pdf -s examples/themes/letter.style.toml'
```

Slow (rebuilds every run); the reusable dev container above is
better for iteration.

## Option E: pre-built binaries

Attached to each
[release](https://github.com/markdoc-rs/markdoc-pdf/releases).
`.github/workflows/release.yml` builds them when a new version in
`Cargo.toml` reaches main, and tags that commit:

| Asset | Platform |
|-------|----------|
| `markdoc-pdf-linux-x86_64` | Linux x86-64, static (musl): any distribution |
| `markdoc-pdf-linux-x86_64-glibc` | Linux x86-64, glibc 2.39 or newer |
| `markdoc-pdf-linux-aarch64` | Linux ARM64, static (musl): any distribution |
| `markdoc-pdf-macos-aarch64` | macOS, Apple Silicon |
| `markdoc-pdf-macos-x86_64` | macOS, Intel |
| `markdoc-pdf-windows-x86_64.exe` | Windows x86-64 |
| `markdoc-pdf-windows-aarch64.exe` | Windows ARM64 |

Each ships a `.sha256` beside it, and `themes.tar.gz` / `themes.zip`
carry the `examples/themes/` catalogue that `--style` needs. Every
asset also has a GitHub build-provenance attestation, which ties it to
the commit and the workflow run that built it:

```sh
gh attestation verify markdoc-pdf-linux-x86_64 --repo markdoc-rs/markdoc-pdf
gh attestation verify markdoc-pdf-linux-x86_64 --repo markdoc-rs/markdoc-pdf \
    --bundle markdoc-pdf-linux-x86_64.sigstore.json
```

The release carries each attestation too, as `<asset>.sigstore.json`
beside the asset; the second form checks against that copy instead of
asking GitHub.

Download, `chmod +x`, drop it on `$PATH`:

```sh
curl -LO https://github.com/markdoc-rs/markdoc-pdf/releases/latest/download/markdoc-pdf-linux-x86_64
chmod +x markdoc-pdf-linux-x86_64
sudo mv markdoc-pdf-linux-x86_64 /usr/local/bin/markdoc-pdf
```

Three Linux builds, for two kinds of machine:

- `markdoc-pdf-linux-x86_64` and `markdoc-pdf-linux-aarch64` (ARM64) are
  statically linked — `static-pie`, zero `NEEDED` entries, fontconfig
  built in — so they run on any distribution whatever its libc: Alpine,
  an older CI image, a current desktop. They are the ones `tw`
  downloads. On a system whose fontconfig is newer than the one built
  in, they print a `Fontconfig warning` for each part of the system's
  font configuration they do not know, and skip that part.
- `markdoc-pdf-linux-x86_64-glibc` uses the system's own glibc and
  fontconfig, so it reads that configuration as the system does and
  says nothing about it. It needs glibc 2.39 or newer (Ubuntu 24.04,
  Debian 13, Fedora 40 and later) and the `fontconfig` package. To have
  `tw` run it instead, point `MARKDOC_PDF` at it.

All of them still need fonts installed; see the note at the top of this
file.

## Updating

Source-installed (Option A): re-run `cargo install --path . --locked`
from the repo. Cargo notices when the source has changed and rebuilds.

Binary-copied (Option B): rebuild and copy again.

## Uninstalling

- Cargo install: `cargo uninstall markdoc-pdf`
- Manual copy: `sudo rm /usr/local/bin/markdoc-pdf`

## Themes

Themes live in `markdoc-pdf/examples/themes/`. They are not bundled
into the binary — pass the path explicitly via `--style`:

```sh
markdoc-pdf -i my.mdoc -o my.pdf -s /path/to/themes/letter.style.toml
```

If you want a stable location for themes on a writer's machine, copy
the whole `themes/` folder somewhere persistent (e.g. `~/.config/markdoc-themes/`)
and reference it from there:

```sh
mkdir -p ~/.config/markdoc-themes
cp -r markdoc-pdf/examples/themes/* ~/.config/markdoc-themes/
markdoc-pdf -i my.mdoc -o my.pdf -s ~/.config/markdoc-themes/letter.style.toml
```

## Troubleshooting

**"command not found: markdoc-pdf"** — the binary isn't on `$PATH`.
For Option A check `~/.cargo/bin` is on `$PATH`; for Option B check
the directory you copied to. On Windows, restart your shell after
the rustup installer finishes so the PATH change takes effect.

**Windows: `link.exe not found`** — the MSVC toolchain needs the
Visual Studio Build Tools. Re-run the rustup installer and let it
install them, or grab the standalone "Build Tools for Visual Studio"
from Microsoft.

**Compile errors during `cargo install`** — Rust toolchain too old.
Update with `rustup update stable`.

**SQLX_OFFLINE warnings during build** — only relevant when building
sibling crates that talk to a database. `markdoc-pdf` itself has no
database dependency; you can ignore the warnings.

**`panicked at … fontique … NoMatch`** — no usable font is installed.
See the note at the top of this file; `apk add font-noto` /
`apt install fonts-noto-core` fixes it. A `FROM scratch` container,
with the static `markdoc-pdf-linux-x86_64` in it, needs the `.ttf`
files copied into `/usr/share/fonts/` (fontconfig's own
`/etc/fonts/fonts.conf` is optional — without it you get `Fontconfig
error: Cannot load default config file` on stderr and a correct PDF).

**Fonts look wrong / missing characters** — the default family chain
leads with Noto Sans and falls through script-specific Noto faces,
DejaVu / Liberation, and the Windows and macOS system families. Only
the ones actually installed participate. Install the Noto face for
the script in question, or point `font_paths` and
`body_font_families` at your own (see `examples/themes/README.md`).
