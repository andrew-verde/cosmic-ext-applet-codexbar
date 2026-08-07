rootdir := ''
prefix := '/usr'
debug := '0'
vendor := '0'
target := if debug == '1' { 'debug' } else { 'release' }
vendor_args := if vendor == '1' { '--frozen --offline' } else { '' }
debug_args := if debug == '1' { '' } else { '--release' }

name := 'cosmic-ext-applet-codexbar'
appid := 'dev.andrewgreen.codexbar'

targetdir := env('CARGO_TARGET_DIR', 'target')
sharedir := rootdir + prefix + '/share'
iconsdir := sharedir + '/icons/hicolor/scalable/apps'
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

_install_icon:
    install -Dm0644 'data/icons/scalable/apps/{{appid}}-symbolic.svg' {{iconsdir}}/{{appid}}-symbolic.svg

_install_desktop:
    install -Dm0644 'data/{{appid}}.desktop' {{sharedir}}/applications/{{appid}}.desktop

_install_bin:
    install -Dm0755 {{targetdir}}/{{target}}/{{name}} {{bindir}}/{{name}}

# Installs files into the system
install: _install_icon _install_desktop _install_bin

# Uninstalls the applet from the system
uninstall:
    rm -f {{bindir}}/{{name}}
    rm -f {{iconsdir}}/{{appid}}-symbolic.svg
    rm -f {{sharedir}}/applications/{{appid}}.desktop

# Vendor Cargo dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor | head -n -1 > .cargo/config
    echo 'directory = "vendor"' >> .cargo/config
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
[private]
vendor-extract:
    rm -rf vendor
    tar pxf vendor.tar
