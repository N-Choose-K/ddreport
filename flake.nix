{
  description = "One-page drive health report PDF from SMART data";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        default = ddreport;
        ddreport = pkgs.rustPlatform.buildRustPackage {
          pname = "ddreport";
          version = (pkgs.lib.importTOML ./Cargo.toml).package.version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.makeWrapper ];

          # The checked-in linker override targets a developer Mac; the nix
          # stdenv supplies its own cc.
          postPatch = "rm -f .cargo/config.toml";

          # smartctl is found on PATH at runtime (or via DDREPORT_SMARTCTL).
          postInstall = ''
            wrapProgram $out/bin/ddreport \
              --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.smartmontools ]}
          '';

          meta = {
            description = "One-page drive health report PDF from SMART data";
            homepage = "https://github.com/nortagawa/ddreport";
            mainProgram = "ddreport";
          };
        };
      });
    };
}
