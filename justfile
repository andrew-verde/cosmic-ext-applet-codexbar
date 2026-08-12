rootdir := ''
prefix := '/usr'
debug := '0'
vendor := '0'
target := if debug == '1' { 'debug' } else { 'release' }
vendor_args := if vendor == '1' { '--frozen --offline' } else { '' }
debug_args := if debug == '1' { '' } else { '--release' }

name := 'cosmic-ext-applet-codexbar'
appid := 'io.github.andrew_verde.cosmic-ext-applet-codexbar'

targetdir := env('CARGO_TARGET_DIR', 'target')
sharedir := rootdir + prefix + '/share'
iconsdir := sharedir + '/icons/hicolor/scalable/apps'
metainfodir := sharedir + '/metainfo'
bindir := rootdir + prefix + '/bin'

default: run

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

run:
    cargo run --release

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles with release profile with wgpu disabled
build-no-wgpu *args: (build-debug '--release --no-default-features' args)

# Compile with a vendored tarball
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Runs the unit tests
test:
    cargo test

# Re-vendors the provider icons from CodexBar and regenerates src/icons.rs
update-icons:
    python3 tools/update-icons.py

_install_icon:
    install -Dm0644 'data/icons/scalable/apps/{{appid}}-symbolic.svg' {{iconsdir}}/{{appid}}-symbolic.svg
    install -Dm0644 'data/icons/scalable/apps/{{appid}}.svg' {{iconsdir}}/{{appid}}.svg

_install_desktop:
    install -Dm0644 'data/{{appid}}.desktop' {{sharedir}}/applications/{{appid}}.desktop

_install_metainfo:
    install -Dm0644 'data/{{appid}}.metainfo.xml' {{metainfodir}}/{{appid}}.metainfo.xml

_install_bin:
    install -Dm0755 {{targetdir}}/{{target}}/{{name}} {{bindir}}/{{name}}

# Installs files into the system
install: _install_icon _install_desktop _install_metainfo _install_bin

# Uninstalls the applet from the system
uninstall:
    rm -f {{bindir}}/{{name}}
    rm -f {{iconsdir}}/{{appid}}-symbolic.svg
    rm -f {{iconsdir}}/{{appid}}.svg
    rm -f {{sharedir}}/applications/{{appid}}.desktop
    rm -f {{metainfodir}}/{{appid}}.metainfo.xml

# Vendor Cargo dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
[private]
vendor-extract:
    rm -rf vendor
    tar pxf vendor.tar
