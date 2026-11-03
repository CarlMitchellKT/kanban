{
  description = "A basic Rust kanban board";

  inputs = {
    nixpkgs.url = "https://channels.nixos.org/nixpkgs-unstable/nixexprs.tar.zst";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f: nixpkgs.lib.genAttrs systems (system: f system nixpkgs.legacyPackages.${system});

      # Libraries that winit/wgpu load with dlopen at runtime, so they are not found by the
      # linker and must be on the library path of the binary.
      runtimeLibs =
        pkgs:
        with pkgs;
        [
          wayland
          libxkbcommon
          vulkan-loader
          libGL
          libx11
          libxcursor
          libxi
          libxrandr
        ];
    in
    {
      packages = forAllSystems (
        system: pkgs: {
          kanban = pkgs.rustPlatform.buildRustPackage {
            pname = "kanban";
            version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
            src = pkgs.lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.makeWrapper
            ];
            buildInputs = runtimeLibs pkgs;

            postFixup = ''
              patchelf --add-rpath ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)} $out/bin/kanban

              # A Nix-built binary cannot load the host's GPU drivers on non-NixOS systems (they
              # are linked against the host glibc). Unless the user picked a Vulkan driver or we
              # are on NixOS (which provides /run/opengl-driver), fall back to Mesa's software
              # Vulkan driver (lavapipe) so the app always starts.
              wrapProgram $out/bin/kanban --run '
                if [ -z "''${VK_ICD_FILENAMES:-}" ] && [ -z "''${VK_DRIVER_FILES:-}" ] \
                   && [ ! -d /run/opengl-driver ]; then
                  export VK_ICD_FILENAMES=${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.${pkgs.stdenv.hostPlatform.parsed.cpu.name}.json
                fi
              '
            '';

            meta = {
              description = "A kanban board with nested tasks and dependencies";
              mainProgram = "kanban";
              platforms = systems;
            };
          };
          default = self.packages.${system}.kanban;
        }
      );

      apps = forAllSystems (
        system: _: {
          default = {
            type = "app";
            program = "${self.packages.${system}.kanban}/bin/kanban";
          };
        }
      );

      devShells = forAllSystems (
        system: pkgs: {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.kanban ];
            packages = with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
            ];
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        }
      );

      formatter = forAllSystems (_: pkgs: pkgs.nixfmt);
    };
}
