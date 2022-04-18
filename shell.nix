let
  rustOverlay = import (builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/0bec2858fd0cc82768b99b5c0c095711324f3ff2.tar.gz");
  nixpkgs = import <nixpkgs> { overlays = [ rustOverlay ]; };
in
with nixpkgs;
stdenv.mkDerivation {
  name = "rust-shell";
  buildInputs = with pkgs; [
    (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
    cargo-edit
    cargo-expand
    cargo-udeps
    hyperfine
  ];
}
