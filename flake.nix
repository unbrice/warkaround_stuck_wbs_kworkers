{
  description = "Monitors kworker/inode_switch_wbs threads and triggers sync if stuck";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    flake-utils,
    advisory-db,
    ...
  }:
    (flake-utils.lib.eachSystem ["x86_64-linux" "aarch64-linux"] (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;
        craneLib = crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;
        commonArgs = {
          inherit src;
          strictDeps = true;
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        stuck_writeback_workaround = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            doCheck = false;
            meta = {
              mainProgram = "stuck_writeback_workaround";
            };
          }
        );
      in {
        checks = {
          inherit stuck_writeback_workaround;
          stuck_writeback_workaround-clippy = craneLib.cargoClippy (commonArgs
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            });
          stuck_writeback_workaround-fmt = craneLib.cargoFmt {inherit src;};
          stuck_writeback_workaround-audit = craneLib.cargoAudit {inherit src advisory-db;};
          stuck_writeback_workaround-nextest = craneLib.cargoNextest (commonArgs
            // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
            });
        };

        packages = {
          stuck-writeback-workaround = stuck_writeback_workaround;
          default = stuck_writeback_workaround;
        };

        apps.default =
          flake-utils.lib.mkApp {
            drv = stuck_writeback_workaround;
          }
          // {meta = stuck_writeback_workaround.meta;};

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};
        };
      }
    ))
    // {
      nixosModules.default = {
        config,
        lib,
        pkgs,
        ...
      }: let
        cfg = config.services.stuck-writeback-workaround;
      in {
        options.services.stuck-writeback-workaround = {
          enable = lib.mkEnableOption "stuck-writeback-workaround";
          debug = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "Enable debug logging.";
          };
          package = lib.mkOption {
            type = lib.types.package;
            default = self.packages.${pkgs.system}.stuck-writeback-workaround;
            description = "The stuck-writeback-workaround package to use.";
          };
          processGlob = lib.mkOption {
            type = lib.types.str;
            default = "kworker/*inode_switch_wbs";
            description = "Glob pattern for the kworker comm field.";
          };
          runtimeThreshold = lib.mkOption {
            type = lib.types.str;
            default = "30s";
            description = "How long a matching worker can run before a sync is triggered.";
          };
        };
        config = lib.mkIf (cfg.enable && pkgs.stdenv.isLinux) {
          systemd.services.stuck-writeback-workaround = {
            description = "Monitors kworker/inode_switch_wbs threads and triggers sync if they get stuck";
            after = ["network.target"];
            wantedBy = ["multi-user.target"];
            serviceConfig = {
              Type = "simple";
              ExecStart = "${lib.getExe cfg.package} --verbose${lib.optionalString cfg.debug " --debug"} --process-glob '${cfg.processGlob}' --runtime-threshold '${cfg.runtimeThreshold}' --no-timestamps";
              Restart = "always";
            };
          };
        };
      };
    };
}
