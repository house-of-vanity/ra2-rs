{ pkgs ? import <nixpkgs> {} }:
let
  libraries = with pkgs; [
    vulkan-loader wayland libxkbcommon alsa-lib
    libx11 libxcursor libxi libxrandr
  ];
in pkgs.mkShell {
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = libraries;
  shellHook = ''
    export PHYSICS_DEMO_SHELL=1
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath libraries}:/run/opengl-driver/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export CARGO_HOME="$PWD/.cargo-home"
    export CARGO_BUILD_JOBS="''${CARGO_BUILD_JOBS:-4}"
  '';
}
