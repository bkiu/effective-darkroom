{
  description = "Effective Darkroom - a Lightroom-style RAW/JPEG library and editor";

  inputs = {
    nixpkgs.url = "flake:nixpkgs";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            pkg-config
            clang
          ];

          buildInputs = with pkgs; [
            libraw
            libxkbcommon
            wayland
            libGL
            vulkan-loader
            libx11
            libxcursor
            libxrandr
            libxi
          ];

          env = {
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            # winit/wgpu dlopen these at runtime; they are not on the default
            # search path on NixOS.
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
              libxkbcommon
              wayland
              libGL
              vulkan-loader
              libx11
              libxcursor
              libxrandr
              libxi
            ]);
          };
        };
      });
    };
}
