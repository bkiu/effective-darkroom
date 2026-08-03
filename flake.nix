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
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "effective-darkroom";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            makeWrapper
            copyDesktopItems
          ];

          buildInputs = with pkgs; [
            libraw
            libgphoto2
          ];

          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "effective-darkroom";
              desktopName = "Effective Darkroom";
              comment = "RAW/JPEG library and editor";
              exec = "effective-darkroom";
              icon = "effective-darkroom";
              categories = [ "Graphics" "Photography" ];
              startupWMClass = "effective-darkroom";
            })
          ];

          postInstall = ''
            install -Dm444 assets/icon.png \
              $out/share/icons/hicolor/256x256/apps/effective-darkroom.png
          '';

          # winit/wgpu dlopen these at runtime; they are not on the default
          # search path on NixOS.
          postFixup = ''
            wrapProgram $out/bin/effective-darkroom \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (with pkgs; [
                libxkbcommon
                wayland
                libGL
                vulkan-loader
                libx11
                libxcursor
                libxrandr
                libxi
              ])}
          '';
        };
      });

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
            libgphoto2
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
